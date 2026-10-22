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
    pub input_id: i32,
    pub expected_draft_version_id: Option<i32>,
}
pub struct RemoveModelInputTool {
    services: Arc<ToolServices>,
}
impl RemoveModelInputTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for RemoveModelInputTool {
    const NAME: &'static str = "remove_model_input";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按当前版本内 inputId 移除误传输入并派生新版本。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"inputId":{"type":"integer"},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &["inputId", "expectedDraftVersionId"],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let v = self
            .services
            .model_patch
            .remove_input(
                &self.services.principal,
                self.services.session_id,
                a.input_id,
                a.expected_draft_version_id,
            )
            .await?;
        Ok(json!({"versionId":v.id,"status":v.status}))
    }
}
