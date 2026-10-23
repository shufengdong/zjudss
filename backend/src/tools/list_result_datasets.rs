use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};
use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub run_id: i32,
}
pub struct ListResultDatasetsTool {
    services: Arc<ToolServices>,
}
impl ListResultDatasetsTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for ListResultDatasetsTool {
    const NAME: &'static str = "list_result_datasets";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "列出指定仿真运行仍然存在的数据集、列 schema、血缘与已保存视图。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(json!({"runId":{"type":"integer"}}), &["runId"])
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let rows = self
            .services
            .datasets
            .list(&self.services.principal, args.run_id)
            .await
            .map_err(|e| DomainToolError::new("dataset_error", e.to_string()))?;
        serde_json::to_value(rows)
            .map_err(|e| DomainToolError::new("serialization_error", e.to_string()))
    }
}
