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
pub struct Args {}
pub struct InspectWorkspaceTool {
    services: Arc<ToolServices>,
}
impl InspectWorkspaceTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for InspectWorkspaceTool {
    const NAME: &'static str = "inspect_workspace";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "检查当前会话工作区的 active/draft、稳定 inputId、文件名、映射证据、缺失项与诊断。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(json!({}), &[])
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, _: Args) -> Result<Value, Self::Error> {
        let record = self
            .services
            .versions
            .workspace(&self.services.principal, self.services.session_id)
            .await?;
        let active=record.active.map(|version|json!({"id":version.version.id,"contentHash":version.version.content_hash,"tables":version.dataframes.into_iter().map(|table|json!({"tableName":table.table_name,"rowCount":table.row_count,"contentHash":table.content_hash})).collect::<Vec<_>>() }));
        let draft=record.draft.map(|draft|json!({"id":draft.draft.id,"revision":draft.draft.revision,"baseVersionId":draft.draft.base_version_id,"diagnostics":serde_json::from_str::<Value>(&draft.draft.diagnostics_json).unwrap_or(Value::Null),"tables":draft.dataframes.into_iter().map(|table|json!({"tableName":table.table_name,"rowCount":table.row_count,"contentHash":table.content_hash})).collect::<Vec<_>>() }));
        Ok(json!({"workspaceId":record.workspace.id,"active":active,"draft":draft}))
    }
}
