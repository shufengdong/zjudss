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
    pub example_id: String,
    pub engine_resource_snapshot: String,
    pub expected_draft_version_id: Option<i32>,
}
pub struct ImportExampleTool {
    services: Arc<ToolServices>,
}
impl ImportExampleTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for ImportExampleTool {
    const NAME: &'static str = "import_example";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "把当前引擎快照中的案例复制到工作区并发布不可变版本；要求快照和预期基线。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"exampleId":{"type":"string"},"engineResourceSnapshot":{"type":"string"},"expectedDraftVersionId":{"type":["integer","null"]}}),
            &[
                "exampleId",
                "engineResourceSnapshot",
                "expectedDraftVersionId",
            ],
        )
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let version = self
            .services
            .example_import
            .import(
                &self.services.principal,
                self.services.session_id,
                &args.example_id,
                &args.engine_resource_snapshot,
                args.expected_draft_version_id,
            )
            .await?;
        Ok(
            json!({"versionId":version.id.to_string(),"status":version.status,"contentHash":version.content_hash}),
        )
    }
}
