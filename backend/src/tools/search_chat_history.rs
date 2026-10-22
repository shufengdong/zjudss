use crate::{
    db::repositories::SearchFilters,
    tools::{
        ToolServices,
        support::{DomainToolError, object_schema},
    },
};
use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    query: String,
    study_type: Option<String>,
    status: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
}
pub struct SearchChatHistoryTool {
    services: Arc<ToolServices>,
}
impl SearchChatHistoryTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for SearchChatHistoryTool {
    const NAME: &'static str = "search_chat_history";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "在当前用户历史会话中搜索中文关键词、标题、附件和任务摘要，最多返回 5 个稳定 session ID 候选。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"query":{"type":"string"},"studyType":{"type":["string","null"]},"status":{"type":["string","null"]},"fromMs":{"type":["integer","null"]},"toMs":{"type":["integer","null"]}}),
            &["query"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let hits = self
            .services
            .history
            .search_sessions(
                &self.services.principal,
                a.query,
                SearchFilters {
                    study_type: a.study_type,
                    status: a.status,
                    from_ms: a.from_ms,
                    to_ms: a.to_ms,
                },
                5,
            )
            .await
            .map_err(|e| DomainToolError::new(e.code, e.to_string()))?;
        serde_json::to_value(hits)
            .map_err(|e| DomainToolError::new("serialization_error", e.to_string()))
    }
}
