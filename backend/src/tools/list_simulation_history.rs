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
    pub session_id: Option<i32>,
    pub limit: Option<i64>,
}
pub struct ListSimulationHistoryTool {
    services: Arc<ToolServices>,
}
impl ListSimulationHistoryTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for ListSimulationHistoryTool {
    const NAME: &'static str = "list_simulation_history";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "列出当前或指定已授权会话最近的仿真运行，供“上次”“修改前”等指代消歧。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"sessionId":{"type":["integer","null"]},"limit":{"type":["integer","null"]}}),
            &[],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let rows = self
            .services
            .runs
            .list(
                &self.services.principal,
                a.session_id.unwrap_or(self.services.session_id),
                a.limit.unwrap_or(10),
            )
            .await
            .map_err(|e| DomainToolError::new("run_error", e.to_string()))?;
        Ok(Value::Array(rows.into_iter().map(|run|json!({"runId":run.id.to_string(),"status":run.status,"modelVersionId":run.model_version_id.map(|id|id.to_string()),"scriptResourceId":run.script_resource_id.to_string(),"createdAt":crate::application::ms_to_iso(run.created_at_ms)})).collect()))
    }
}
