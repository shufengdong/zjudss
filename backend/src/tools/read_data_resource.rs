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
    pub resource_id: String,
    #[serde(default)]
    pub columns: Vec<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}
pub struct ReadDataResourceTool {
    services: Arc<ToolServices>,
}
impl ReadDataResourceTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for ReadDataResourceTool {
    const NAME: &'static str = "read_data_resource";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "读取当前会话中指定普通数据资源的元数据和有界行；只接受稳定 resourceId、列投影和 cursor。"
            .into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"resourceId":{"type":"string"},"columns":{"type":"array","items":{"type":"string"},"maxItems":32},"limit":{"type":["integer","null"],"minimum":1,"maximum":100},"cursor":{"type":["string","null"]}}),
            &["resourceId"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let metadata = self
            .services
            .data_resources
            .get(&self.services.principal, &args.resource_id)
            .await
            .map_err(|e| DomainToolError::new(e.code(), e.to_string()))?;
        if metadata.session_id != self.services.session_id.to_string() {
            return Err(DomainToolError::new("not_found", "资源不存在"));
        }
        let rows = self
            .services
            .data_resources
            .rows(
                &self.services.principal,
                &args.resource_id,
                Some(args.limit.unwrap_or(20)),
                args.cursor.as_deref(),
                args.columns,
            )
            .await
            .map_err(|e| DomainToolError::new(e.code(), e.to_string()))?;
        Ok(json!({"resource":metadata,"page":rows}))
    }
}
