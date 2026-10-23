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
    pub version_id: i32,
    pub expected_draft_version_id: Option<i32>,
}
pub struct ActivateModelVersionTool {
    services: Arc<ToolServices>,
}
impl ActivateModelVersionTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for ActivateModelVersionTool {
    const NAME: &'static str = "activate_model_version";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "激活当前 ready draft；expectedDraftVersionId 必须是当前 draft 的版本 ID，通常与要激活的 versionId 相同，不是创建该 draft 时的旧基线。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"versionId":{"type":"integer"},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &["versionId", "expectedDraftVersionId"],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let r = self
            .services
            .workspaces
            .get(&self.services.principal.id, self.services.session_id)
            .await?;
        if r.workspace
            .draft_version_id
            .or(r.workspace.active_version_id)
            .is_some()
            && a.expected_draft_version_id.is_none()
        {
            return Err(DomainToolError::new(
                "expected_version_required",
                "expectedDraftVersionId 必填",
            ));
        }
        let r = self
            .services
            .workspaces
            .activate_checked(
                &self.services.principal.id,
                r.workspace.id,
                a.version_id,
                a.expected_draft_version_id,
            )
            .await?;
        Ok(json!({"activeVersionId":r.workspace.active_version_id}))
    }
}
pub struct RollbackModelVersionTool {
    services: Arc<ToolServices>,
}
impl RollbackModelVersionTool {
    pub fn new(s: Arc<ToolServices>) -> Self {
        Self { services: s }
    }
}
impl Tool for RollbackModelVersionTool {
    const NAME: &'static str = "rollback_model_version";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "把 active 指针回退到已有 ready 历史版本；不修改历史任务。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"versionId":{"type":"integer"},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &["versionId", "expectedDraftVersionId"],
        )
    }
    async fn call(&self, _: &mut ToolContext, a: Args) -> Result<Value, Self::Error> {
        let r = self
            .services
            .workspaces
            .get(&self.services.principal.id, self.services.session_id)
            .await?;
        if r.workspace
            .draft_version_id
            .or(r.workspace.active_version_id)
            .is_some()
            && a.expected_draft_version_id.is_none()
        {
            return Err(DomainToolError::new(
                "expected_version_required",
                "expectedDraftVersionId 必填",
            ));
        }
        let r = self
            .services
            .workspaces
            .rollback_checked(
                &self.services.principal.id,
                r.workspace.id,
                a.version_id,
                a.expected_draft_version_id,
            )
            .await?;
        Ok(
            json!({"activeVersionId":r.workspace.active_version_id,"draftVersionId":r.workspace.draft_version_id}),
        )
    }
}
