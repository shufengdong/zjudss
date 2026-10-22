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
    session_id: Option<i32>,
    around_message_id: i32,
    radius: Option<i64>,
}
pub struct ReadChatContextTool {
    services: Arc<ToolServices>,
}
impl ReadChatContextTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for ReadChatContextTool {
    const NAME: &'static str = "read_chat_context";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "读取一个已确认历史消息附近的有限上下文；默认当前会话，最多 50 条。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"sessionId":{"type":["integer","null"]},"aroundMessageId":{"type":"integer"},"radius":{"type":["integer","null"]}}),
            &["aroundMessageId"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let rows = self
            .services
            .history
            .read_context(
                &self.services.principal,
                a.session_id.unwrap_or(self.services.session_id),
                a.around_message_id,
                a.radius.unwrap_or(20),
            )
            .await
            .map_err(|e| DomainToolError::new(e.code, e.to_string()))?;
        Ok(Value::Array(rows.into_iter().map(|m|json!({"id":m.id.to_string(),"role":m.role,"kind":m.kind,"content":m.content,"createdAt":crate::application::ms_to_iso(m.created_at_ms)})).collect()))
    }
}
