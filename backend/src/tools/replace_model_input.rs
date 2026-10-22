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
    pub target_input_id: i32,
    pub replacement_input_id: i32,
    pub expected_draft_version_id: Option<i32>,
}
pub struct ReplaceModelInputTool {
    services: Arc<ToolServices>,
}
impl ReplaceModelInputTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for ReplaceModelInputTool {
    const NAME: &'static str = "replace_model_input";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "用同一候选版本中的 replacementInputId 替换 targetInputId，并派生新版本。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"targetInputId":{"type":"integer"},"replacementInputId":{"type":"integer"},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &[
                "targetInputId",
                "replacementInputId",
                "expectedDraftVersionId",
            ],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let v = self
            .services
            .model_patch
            .replace_input(
                &self.services.principal,
                self.services.session_id,
                a.target_input_id,
                a.replacement_input_id,
                a.expected_draft_version_id,
            )
            .await?;
        Ok(json!({"versionId":v.id,"status":v.status}))
    }
}
