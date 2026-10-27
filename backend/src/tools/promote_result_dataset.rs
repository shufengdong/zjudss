use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub dataset_id: i32,
    pub display_name: String,
}
pub struct PromoteResultDatasetTool {
    services: Arc<ToolServices>,
}
impl PromoteResultDatasetTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for PromoteResultDatasetTool {
    const NAME: &'static str = "promote_result_dataset";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "把用户明确指定的单个仿真结果或物化分析 dataset 提升为当前会话的普通数据资源；存在多个候选时必须先让用户选择稳定 datasetId。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"datasetId":{"type":"integer"},"displayName":{"type":"string","minLength":1,"maxLength":200}}),
            &["datasetId", "displayName"],
        )
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let resource = self
            .services
            .data_resources
            .promote_dataset(
                &self.services.principal,
                self.services.session_id,
                args.dataset_id,
                args.display_name,
            )
            .await
            .map_err(|e| DomainToolError::new(e.code(), e.to_string()))?;
        serde_json::to_value(resource)
            .map_err(|e| DomainToolError::new("serialization_error", e.to_string()))
    }
}
