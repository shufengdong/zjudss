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
pub struct Args {
    pub example_id: String,
}
pub struct InspectExampleTool {
    services: Arc<ToolServices>,
}
impl InspectExampleTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for InspectExampleTool {
    const NAME: &'static str = "inspect_example";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按稳定 exampleId 查看案例目录、规范 manifest、声明文件角色与验证诊断；不接受物理路径。"
            .into()
    }
    fn parameters(&self) -> Value {
        object_schema(json!({"example_id":{"type":"string"}}), &["example_id"])
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services
            .engine_snapshot
            .examples
            .iter()
            .find(|e| e.id == args.example_id)
            .map(|example| {
                let files = example
                    .files
                    .iter()
                    .map(|file| {
                        json!({
                            "path": file.virtual_path,
                            "role": file.role,
                            "sha256": file.sha256,
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "snapshotId": self.services.engine_snapshot.id,
                    "example": {
                        "exampleId": example.id,
                        "title": example.title,
                        "categoryPath": example.category_path,
                        "bundlePath": example.bundle_path,
                        "entryPath": example.entry_path,
                        "manifest": example.manifest,
                        "files": files,
                        "treeHash": example.tree_hash,
                        "importable": example.importable,
                        "diagnostics": example.diagnostics,
                    }
                })
            })
            .ok_or_else(|| DomainToolError::new("example_not_found", "未知 exampleId"))
    }
}
