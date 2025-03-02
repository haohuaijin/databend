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

use common_catalog::plan::gen_append_stream_columns;
use common_catalog::plan::StreamColumnMeta;
use common_catalog::table::Table;
use common_exception::Result;
use common_expression::DataSchemaRef;
use common_pipeline_core::Pipeline;
use common_sql::TransformStreamKind;

use crate::pipelines::processors::transforms::TransformAddComputedColumns;
use crate::pipelines::processors::transforms::TransformAddStreamColumns;
use crate::pipelines::processors::TransformResortAddOn;
use crate::pipelines::PipelineBuilder;
use crate::sessions::QueryContext;

/// This file implements append to table pipeline builder.
impl PipelineBuilder {
    pub fn build_fill_missing_columns_pipeline(
        ctx: Arc<QueryContext>,
        pipeline: &mut Pipeline,
        table: Arc<dyn Table>,
        source_schema: DataSchemaRef,
    ) -> Result<()> {
        let table_default_schema = &table.schema().remove_computed_fields();
        let table_computed_schema = &table.schema().remove_virtual_computed_fields();
        let default_schema: DataSchemaRef = Arc::new(table_default_schema.into());
        let computed_schema: DataSchemaRef = Arc::new(table_computed_schema.into());

        // Fill missing default columns and resort the columns.
        if source_schema != default_schema {
            pipeline.add_transform(|transform_input_port, transform_output_port| {
                TransformResortAddOn::try_create(
                    ctx.clone(),
                    transform_input_port,
                    transform_output_port,
                    source_schema.clone(),
                    default_schema.clone(),
                    table.clone(),
                )
            })?;
        }

        // Fill computed columns.
        if default_schema != computed_schema {
            pipeline.add_transform(|transform_input_port, transform_output_port| {
                TransformAddComputedColumns::try_create(
                    ctx.clone(),
                    transform_input_port,
                    transform_output_port,
                    default_schema.clone(),
                    computed_schema.clone(),
                )
            })?;
        }

        // Fill stream columns.
        if table.change_tracking_enabled() {
            let version = table.get_table_info().ident.seq;
            let stream_columns = gen_append_stream_columns();
            pipeline.add_transform(|transform_input_port, transform_output_port| {
                TransformAddStreamColumns::try_create(
                    transform_input_port,
                    transform_output_port,
                    TransformStreamKind::Append(StreamColumnMeta::Append(version)),
                    stream_columns.clone(),
                )
            })?;
        }
        Ok(())
    }
}
