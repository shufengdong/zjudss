use crate::application::ModelPatch;
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
    pub patch: ModelPatch,
    pub expected_draft_version_id: Option<i32>,
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
        "校验 tableKind、唯一 rowKey、from/to 和 unit，并派生不可变模型版本。对 .dss 文本使用保留 tableKind=dss_property（dss_property 不是引擎函数），rowKey.object 指定如 circuit.IEEE13Nodeckt 的对象，changes.<property> 指定唯一属性及 {from,to,unit}；对 .rss 参数使用 tableKind=rss_parameter、rowKey.name 和 changes.value。"
            .into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"inputId":{"type":"integer"},"patch":{"type":"object","properties":{"tableKind":{"type":"string"},"rowKey":{"type":"object"},"changes":{"type":"object"}},"required":["tableKind","rowKey","changes"],"additionalProperties":false},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &["inputId", "patch", "expectedDraftVersionId"],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        if matches!(
            a.patch.table_kind.as_str(),
            "rss_parameter" | "dss_property"
        ) {
            self.services.require_skill("rss-generation").await?;
        }
        let v = self
            .services
            .model_patch
            .modify(
                &self.services.principal,
                self.services.session_id,
                a.input_id,
                a.patch,
                a.expected_draft_version_id,
            )
            .await?;
        Ok(json!({"versionId":v.id,"status":v.status,"contentHash":v.content_hash}))
    }
}
