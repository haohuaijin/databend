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

use common_exception::Result;
use common_license::license::Feature;
use common_license::license_manager::get_license_manager;
use common_sql::plans::CreateStreamPlan;
use common_storages_fuse::TableContext;
use stream_handler::get_stream_handler;

use crate::interpreters::Interpreter;
use crate::pipelines::PipelineBuildResult;
use crate::sessions::QueryContext;

pub struct CreateStreamInterpreter {
    ctx: Arc<QueryContext>,
    plan: CreateStreamPlan,
}

impl CreateStreamInterpreter {
    pub fn try_create(ctx: Arc<QueryContext>, plan: CreateStreamPlan) -> Result<Self> {
        Ok(CreateStreamInterpreter { ctx, plan })
    }
}

#[async_trait::async_trait]
impl Interpreter for CreateStreamInterpreter {
    fn name(&self) -> &str {
        "CreateStreamInterpreter"
    }

    #[async_backtrace::framed]
    async fn execute2(&self) -> Result<PipelineBuildResult> {
        let license_manager = get_license_manager();
        license_manager
            .manager
            .check_enterprise_enabled(self.ctx.get_license_key(), Feature::Stream)?;

        let handler = get_stream_handler();
        let _ = handler
            .do_create_stream(self.ctx.clone(), &self.plan)
            .await?;

        Ok(PipelineBuildResult::create())
    }
}
