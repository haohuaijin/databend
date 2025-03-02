// Copyright 2021 Datafuse Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use common_ast::parser::parse_comma_separated_exprs;
use common_ast::parser::tokenize_sql;
use common_base::base::tokio::sync::Semaphore;
use common_catalog::table::Table;
use common_catalog::table_context::TableContext;
use common_exception::ErrorCode;
use common_exception::Result;
use common_expression::ColumnBuilder;
use common_expression::DataBlock;
use common_expression::DataSchema;
use common_expression::DataSchemaRef;
use common_expression::Scalar;
use common_formats::FastFieldDecoderValues;
use common_formats::FastValuesDecodeFallback;
use common_formats::FastValuesDecoder;
use common_functions::BUILTIN_FUNCTIONS;
use common_pipeline_core::processors::InputPort;
use common_pipeline_core::processors::OutputPort;
use common_pipeline_core::Pipe;
use common_pipeline_sources::AsyncSource;
use common_pipeline_sources::AsyncSourcer;
use common_pipeline_transforms::processors::create_dummy_item;
use common_sql::executor::physical_plans::ReplaceAsyncSourcer;
use common_sql::executor::physical_plans::ReplaceDeduplicate;
use common_sql::executor::physical_plans::ReplaceInto;
use common_sql::executor::physical_plans::ReplaceSelectCtx;
use common_sql::BindContext;
use common_sql::Metadata;
use common_sql::MetadataRef;
use common_sql::NameResolutionContext;
use common_storages_fuse::operations::common::TransformSerializeSegment;
use common_storages_fuse::operations::processors::BroadcastProcessor;
use common_storages_fuse::operations::processors::ReplaceIntoProcessor;
use common_storages_fuse::operations::processors::UnbranchedReplaceIntoProcessor;
use common_storages_fuse::operations::TransformSerializeBlock;
use common_storages_fuse::FuseTable;
use parking_lot::RwLock;

use crate::pipelines::processors::TransformCastSchema;
use crate::pipelines::PipelineBuilder;

impl PipelineBuilder {
    // check if cast needed
    fn check_schema_cast(
        select_schema: Arc<DataSchema>,
        output_schema: Arc<DataSchema>,
    ) -> Result<bool> {
        let cast_needed = select_schema != output_schema;
        Ok(cast_needed)
    }

    // build async sourcer pipeline.
    pub(crate) fn build_async_sourcer(
        &mut self,
        async_sourcer: &ReplaceAsyncSourcer,
    ) -> Result<()> {
        self.main_pipeline.add_source(
            |output| {
                let name_resolution_ctx = NameResolutionContext::try_from(self.settings.as_ref())?;
                let inner = ValueSource::new(
                    async_sourcer.value_data.clone(),
                    self.ctx.clone(),
                    name_resolution_ctx,
                    async_sourcer.schema.clone(),
                    async_sourcer.start,
                );
                AsyncSourcer::create(self.ctx.clone(), output, inner)
            },
            1,
        )?;
        Ok(())
    }

    // build replace into pipeline.
    pub(crate) fn build_replace_into(&mut self, replace: &ReplaceInto) -> Result<()> {
        let ReplaceInto {
            input,
            block_thresholds,
            table_info,
            on_conflicts,
            bloom_filter_column_indexes,
            catalog_info,
            segments,
            block_slots,
            need_insert,
        } = replace;
        let max_threads = self.settings.get_max_threads()?;
        let segment_partition_num = std::cmp::min(segments.len(), max_threads as usize);
        let table = self
            .ctx
            .build_table_by_table_info(catalog_info, table_info, None)?;
        let table = FuseTable::try_from_table(table.as_ref())?;
        let cluster_stats_gen =
            table.get_cluster_stats_gen(self.ctx.clone(), 0, *block_thresholds, None)?;
        self.build_pipeline(input)?;
        // connect to broadcast processor and append transform
        let serialize_block_transform = TransformSerializeBlock::try_create(
            self.ctx.clone(),
            InputPort::create(),
            OutputPort::create(),
            table,
            cluster_stats_gen,
        )?;
        let block_builder = serialize_block_transform.get_block_builder();

        let serialize_segment_transform = TransformSerializeSegment::new(
            self.ctx.clone(),
            InputPort::create(),
            OutputPort::create(),
            table,
            *block_thresholds,
        );
        if !*need_insert {
            if segment_partition_num == 0 {
                return Ok(());
            }
            let broadcast_processor = BroadcastProcessor::new(segment_partition_num);
            self.main_pipeline
                .add_pipe(Pipe::create(1, segment_partition_num, vec![
                    broadcast_processor.into_pipe_item(),
                ]));
            let max_threads = self.settings.get_max_threads()?;
            let io_request_semaphore = Arc::new(Semaphore::new(max_threads as usize));

            let merge_into_operation_aggregators = table.merge_into_mutators(
                self.ctx.clone(),
                segment_partition_num,
                block_builder,
                on_conflicts.clone(),
                bloom_filter_column_indexes.clone(),
                segments,
                block_slots.clone(),
                io_request_semaphore,
            )?;
            self.main_pipeline.add_pipe(Pipe::create(
                segment_partition_num,
                segment_partition_num,
                merge_into_operation_aggregators,
            ));
            return Ok(());
        }

        if segment_partition_num == 0 {
            let dummy_item = create_dummy_item();
            //                      ┌──────────────────────┐            ┌──────────────────┐
            //                      │                      ├──┬────────►│  SerializeBlock  │
            // ┌─────────────┐      │                      ├──┘         └──────────────────┘
            // │ UpsertSource├─────►│ ReplaceIntoProcessor │
            // └─────────────┘      │                      ├──┐         ┌──────────────────┐
            //                      │                      ├──┴────────►│  DummyTransform  │
            //                      └──────────────────────┘            └──────────────────┘
            // wrap them into pipeline, order matters!
            self.main_pipeline.add_pipe(Pipe::create(2, 2, vec![
                serialize_block_transform.into_pipe_item(),
                dummy_item,
            ]));
        } else {
            //                      ┌──────────────────────┐            ┌──────────────────┐
            //                      │                      ├──┬────────►│ SerializeBlock   │
            // ┌─────────────┐      │                      ├──┘         └──────────────────┘
            // │ UpsertSource├─────►│ ReplaceIntoProcessor │
            // └─────────────┘      │                      ├──┐         ┌──────────────────┐
            //                      │                      ├──┴────────►│BroadcastProcessor│
            //                      └──────────────────────┘            └──────────────────┘
            let broadcast_processor = BroadcastProcessor::new(segment_partition_num);
            // wrap them into pipeline, order matters!
            self.main_pipeline
                .add_pipe(Pipe::create(2, segment_partition_num + 1, vec![
                    serialize_block_transform.into_pipe_item(),
                    broadcast_processor.into_pipe_item(),
                ]));
        };

        // 4. connect with MergeIntoOperationAggregators
        if segment_partition_num == 0 {
            let dummy_item = create_dummy_item();
            self.main_pipeline.add_pipe(Pipe::create(2, 2, vec![
                serialize_segment_transform.into_pipe_item(),
                dummy_item,
            ]));
        } else {
            //      ┌──────────────────┐               ┌────────────────┐
            // ────►│  SerializeBlock  ├──────────────►│SerializeSegment│
            //      └──────────────────┘               └────────────────┘
            //
            //      ┌───────────────────┐              ┌──────────────────────┐
            // ────►│                   ├──┬──────────►│MergeIntoOperationAggr│
            //      │                   ├──┘           └──────────────────────┘
            //      │ BroadcastProcessor│
            //      │                   ├──┐           ┌──────────────────────┐
            //      │                   ├──┴──────────►│MergeIntoOperationAggr│
            //      │                   │              └──────────────────────┘
            //      │                   ├──┐
            //      │                   ├──┴──────────►┌──────────────────────┐
            //      └───────────────────┘              │MergeIntoOperationAggr│
            //                                         └──────────────────────┘

            let item_size = segment_partition_num + 1;
            let mut pipe_items = Vec::with_capacity(item_size);
            // setup the dummy transform
            pipe_items.push(serialize_segment_transform.into_pipe_item());

            let max_threads = self.settings.get_max_threads()?;
            let io_request_semaphore = Arc::new(Semaphore::new(max_threads as usize));

            // setup the merge into operation aggregators
            let mut merge_into_operation_aggregators = table.merge_into_mutators(
                self.ctx.clone(),
                segment_partition_num,
                block_builder,
                on_conflicts.clone(),
                bloom_filter_column_indexes.clone(),
                segments,
                block_slots.clone(),
                io_request_semaphore,
            )?;
            assert_eq!(
                segment_partition_num,
                merge_into_operation_aggregators.len()
            );
            pipe_items.append(&mut merge_into_operation_aggregators);

            // extend the pipeline
            assert_eq!(self.main_pipeline.output_len(), item_size);
            assert_eq!(pipe_items.len(), item_size);
            self.main_pipeline
                .add_pipe(Pipe::create(item_size, item_size, pipe_items));
        }
        Ok(())
    }

    // build deduplicate pipeline.
    pub(crate) fn build_deduplicate(&mut self, deduplicate: &ReplaceDeduplicate) -> Result<()> {
        let ReplaceDeduplicate {
            input,
            on_conflicts,
            bloom_filter_column_indexes,
            table_is_empty,
            table_info,
            catalog_info,
            select_ctx,
            table_level_range_index,
            table_schema,
            need_insert,
            delete_when,
        } = deduplicate;

        let tbl = self
            .ctx
            .build_table_by_table_info(catalog_info, table_info, None)?;
        let table = FuseTable::try_from_table(tbl.as_ref())?;
        self.build_pipeline(input)?;
        let mut delete_column_idx = 0;
        let mut opt_modified_schema = None;
        if let Some(ReplaceSelectCtx {
            select_column_bindings,
            select_schema,
        }) = select_ctx
        {
            PipelineBuilder::build_result_projection(
                &self.func_ctx,
                input.output_schema()?,
                select_column_bindings,
                &mut self.main_pipeline,
                false,
            )?;

            let mut target_schema: DataSchema = table_schema.clone().into();
            if let Some((_, delete_column)) = delete_when {
                delete_column_idx = select_schema.index_of(delete_column.as_str())?;
                let delete_column = select_schema.field(delete_column_idx).clone();
                target_schema
                    .fields
                    .insert(delete_column_idx, delete_column);
                opt_modified_schema = Some(Arc::new(target_schema.clone()));
            }
            let target_schema = Arc::new(target_schema.clone());
            if target_schema.fields().len() != select_schema.fields().len() {
                return Err(ErrorCode::BadArguments(
                    "The number of columns in the target table is different from the number of columns in the SELECT clause",
                ));
            }
            if Self::check_schema_cast(select_schema.clone(), target_schema.clone())? {
                self.main_pipeline.add_transform(
                    |transform_input_port, transform_output_port| {
                        TransformCastSchema::try_create(
                            transform_input_port,
                            transform_output_port,
                            select_schema.clone(),
                            target_schema.clone(),
                            self.func_ctx.clone(),
                        )
                    },
                )?;
            }
        }

        Self::build_fill_missing_columns_pipeline(
            self.ctx.clone(),
            &mut self.main_pipeline,
            tbl.clone(),
            Arc::new(table_schema.clone().into()),
        )?;

        let _ = table.cluster_gen_for_append(
            self.ctx.clone(),
            &mut self.main_pipeline,
            table.get_block_thresholds(),
            opt_modified_schema,
        )?;
        // 1. resize input to 1, since the UpsertTransform need to de-duplicate inputs "globally"
        self.main_pipeline.try_resize(1)?;

        // 2. connect with ReplaceIntoProcessor

        //                      ┌──────────────────────┐
        //                      │                      ├──┐
        // ┌─────────────┐      │                      ├──┘
        // │ UpsertSource├─────►│ ReplaceIntoProcessor │
        // └─────────────┘      │                      ├──┐
        //                      │                      ├──┘
        //                      └──────────────────────┘
        // NOTE: here the pipe items of last pipe are arranged in the following order
        // (0) -> output_port_append_data
        // (1) -> output_port_merge_into_action
        //    the "downstream" is supposed to be connected with a processor which can process MergeIntoOperations
        //    in our case, it is the broadcast processor
        let delete_when = if let Some((remote_expr, delete_column)) = delete_when {
            Some((
                remote_expr.as_expr(&BUILTIN_FUNCTIONS),
                delete_column.clone(),
            ))
        } else {
            None
        };
        let cluster_keys = table.cluster_keys(self.ctx.clone());
        if *need_insert {
            let replace_into_processor = ReplaceIntoProcessor::create(
                self.ctx.clone(),
                on_conflicts.clone(),
                cluster_keys,
                bloom_filter_column_indexes.clone(),
                table_schema.as_ref(),
                *table_is_empty,
                table_level_range_index.clone(),
                delete_when.map(|(expr, _)| (expr, delete_column_idx)),
            )?;
            self.main_pipeline
                .add_pipe(replace_into_processor.into_pipe());
        } else {
            let replace_into_processor = UnbranchedReplaceIntoProcessor::create(
                self.ctx.as_ref(),
                on_conflicts.clone(),
                cluster_keys,
                bloom_filter_column_indexes.clone(),
                table_schema.as_ref(),
                *table_is_empty,
                table_level_range_index.clone(),
                delete_when.map(|_| delete_column_idx),
            )?;
            self.main_pipeline
                .add_pipe(replace_into_processor.into_pipe());
        }
        Ok(())
    }
}

pub struct ValueSource {
    data: String,
    ctx: Arc<dyn TableContext>,
    name_resolution_ctx: NameResolutionContext,
    bind_context: BindContext,
    schema: DataSchemaRef,
    metadata: MetadataRef,
    start: usize,
    is_finished: bool,
}

#[async_trait::async_trait]
impl AsyncSource for ValueSource {
    const NAME: &'static str = "ValueSource";
    const SKIP_EMPTY_DATA_BLOCK: bool = true;

    #[async_trait::unboxed_simple]
    #[async_backtrace::framed]
    async fn generate(&mut self) -> Result<Option<DataBlock>> {
        if self.is_finished {
            return Ok(None);
        }

        let format = self.ctx.get_format_settings()?;
        let field_decoder = FastFieldDecoderValues::create_for_insert(format);

        let mut values_decoder = FastValuesDecoder::new(&self.data, &field_decoder);
        let estimated_rows = values_decoder.estimated_rows();

        let mut columns = self
            .schema
            .fields()
            .iter()
            .map(|f| ColumnBuilder::with_capacity(f.data_type(), estimated_rows))
            .collect::<Vec<_>>();

        values_decoder.parse(&mut columns, self).await?;

        let columns = columns
            .into_iter()
            .map(|col| col.build())
            .collect::<Vec<_>>();
        let block = DataBlock::new_from_columns(columns);
        self.is_finished = true;
        Ok(Some(block))
    }
}

#[async_trait::async_trait]
impl FastValuesDecodeFallback for ValueSource {
    async fn parse_fallback(&self, sql: &str) -> Result<Vec<Scalar>> {
        let res: Result<Vec<Scalar>> = try {
            let settings = self.ctx.get_settings();
            let sql_dialect = settings.get_sql_dialect()?;
            let tokens = tokenize_sql(sql)?;
            let mut bind_context = self.bind_context.clone();
            let metadata = self.metadata.clone();

            let exprs = parse_comma_separated_exprs(&tokens[1..tokens.len()], sql_dialect)?;
            bind_context
                .exprs_to_scalar(
                    exprs,
                    &self.schema,
                    self.ctx.clone(),
                    &self.name_resolution_ctx,
                    metadata,
                )
                .await?
        };
        res.map_err(|mut err| {
            // The input for ValueSource is a sub-section of the original SQL. This causes
            // the error span to have an offset, so we adjust the span accordingly.
            if let Some(span) = err.span() {
                err = err.set_span(Some(
                    (span.start() + self.start..span.end() + self.start).into(),
                ));
            }
            err
        })
    }
}

impl ValueSource {
    pub fn new(
        data: String,
        ctx: Arc<dyn TableContext>,
        name_resolution_ctx: NameResolutionContext,
        schema: DataSchemaRef,
        start: usize,
    ) -> Self {
        let bind_context = BindContext::new();
        let metadata = Arc::new(RwLock::new(Metadata::default()));

        Self {
            data,
            ctx,
            name_resolution_ctx,
            schema,
            bind_context,
            metadata,
            start,
            is_finished: false,
        }
    }
}
