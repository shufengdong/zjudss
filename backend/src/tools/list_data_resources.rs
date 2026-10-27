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
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}
pub struct ListDataResourcesTool {
    services: Arc<ToolServices>,
}
impl ListDataResourcesTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for ListDataResourcesTool {
    const NAME: &'static str = "list_data_resources";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "列出当前会话可见的普通 DataFrame 数据资源、alias、schema、内容 hash 和来源；返回稳定 resource ID，不返回路径。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"cursor":{"type":["string","null"]},"limit":{"type":["integer","null"],"minimum":1,"maximum":100}}),
            &[],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let page = self
            .services
            .data_resources
            .list(
                &self.services.principal,
                self.services.session_id,
                args.cursor,
                args.limit.unwrap_or(50),
            )
            .await
            .map_err(|e| DomainToolError::new(e.code(), e.to_string()))?;
        serde_json::to_value(page)
            .map_err(|e| DomainToolError::new("serialization_error", e.to_string()))
    }
}
