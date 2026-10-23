use std::sync::Arc;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Deserialize)]
pub struct Args {}

pub struct GetStorageUsageTool {
    services: Arc<ToolServices>,
}

impl GetStorageUsageTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for GetStorageUsageTool {
    const NAME: &'static str = "get_storage_usage";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "返回当前授权存储 scope 的 artifact 数量、逻辑字节、限制和告警状态。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(json!({}), &[])
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, _: &mut ToolContext, _: Args) -> Result<Value, Self::Error> {
        let usage = self
            .services
            .artifact_service
            .usage_for(self.services.principal.storage_scope_id)
            .await
            .map_err(|error| DomainToolError::new("storage_usage_error", error.to_string()))?;
        serde_json::to_value(usage)
            .map_err(|error| DomainToolError::new("serialization_error", error.to_string()))
    }
}
