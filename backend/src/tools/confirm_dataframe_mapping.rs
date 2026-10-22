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
    pub table_kind: String,
    pub expected_draft_version_id: Option<i32>,
}
pub struct ConfirmDataframeMappingTool {
    services: Arc<ToolServices>,
}
impl ConfirmDataframeMappingTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for ConfirmDataframeMappingTool {
    const NAME: &'static str = "confirm_dataframe_mapping";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "用版本内 inputId 固化 DataFrame 类型映射并派生新版本；保留原推断证据。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"inputId":{"type":"integer"},"tableKind":{"type":"string"},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &["inputId", "tableKind", "expectedDraftVersionId"],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let v = self
            .services
            .model_patch
            .confirm_mapping(
                &self.services.principal,
                self.services.session_id,
                a.input_id,
                a.table_kind,
                a.expected_draft_version_id,
            )
            .await?;
        Ok(json!({"versionId":v.id,"status":v.status}))
    }
}
