use crate::application::ModelPatch;
use crate::db::models::DraftRef;
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
    pub table_name: String,
    pub patch: ModelPatch,
    pub expected_draft: Option<DraftRef>,
}
pub struct ModifyModelTool {
    services: Arc<ToolServices>,
}
impl ModifyModelTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for ModifyModelTool {
    const NAME: &'static str = "modify_model";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按唯一 rowKey 修改草稿中的一个规范 Parquet DataFrame。必须提供当前 expectedDraft（无草稿时为 null）；修改只产生新的草稿 revision，用户点击“生成版本”后才发布。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"tableName":{"type":"string"},"patch":{"type":"object","properties":{"rowKey":{"type":"object"},"changes":{"type":"object"}},"required":["rowKey","changes"],"additionalProperties":false},"expectedDraft":{"anyOf":[{"type":"object","properties":{"id":{"type":"integer"},"revision":{"type":"integer"}},"required":["id","revision"],"additionalProperties":false},{"type":"null"}]}}),
            &["tableName", "patch", "expectedDraft"],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let v = self
            .services
            .model_patch
            .modify(
                &self.services.principal,
                self.services.session_id,
                a.table_name,
                a.patch,
                a.expected_draft,
            )
            .await?;
        Ok(json!({"draftId":v.id.to_string(),"revision":v.revision}))
    }
}
