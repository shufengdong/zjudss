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
        "检查当前会话工作区的 active/draft、稳定 inputId、映射证据、缺失项与诊断。".into()
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
            .workspaces
            .get_or_create(&self.services.principal.access, self.services.session_id)
            .await?;
        let mut versions = Vec::new();
        for (scope, version) in [("active", record.active), ("draft", record.draft)] {
            if let Some(version) = version {
                let manifest: Value =
                    serde_json::from_str(&version.manifest_json).unwrap_or_else(|_| json!({}));
                let inputs=self.services.workspaces.inputs(&self.services.principal.access,version.id).await?.into_iter().map(|(input,binding)|json!({"inputId":input.id,"role":input.role,"tableKind":binding.as_ref().and_then(|b|b.table_kind.clone()),"resolution":binding.as_ref().map(|b|b.resolution.clone()),"evidence":binding.and_then(|b|serde_json::from_str::<Value>(&b.evidence_json).ok())})).collect::<Vec<_>>();
                versions.push(json!({"scope":scope,"versionId":version.id,"status":version.status,"sourceKind":version.source_kind,"contentHash":version.content_hash,"inputs":inputs,"missing":manifest.get("missing").cloned().unwrap_or_else(||json!([])),"diagnostics":manifest.get("diagnostics").cloned().unwrap_or_else(||json!([]))}));
            }
        }
        Ok(json!({"workspaceId":record.workspace.id,"versions":versions}))
    }
}
