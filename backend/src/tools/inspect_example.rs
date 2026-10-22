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
        "按稳定 exampleId 查看案例摘要、验证状态和资源要求；不接受路径。".into()
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
            .map(|e| json!({"snapshotId":self.services.engine_snapshot.id,"example":e}))
            .ok_or_else(|| DomainToolError::new("example_not_found", "未知 exampleId"))
    }
}
