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
    pub query: String,
    #[serde(default)]
    pub max_chars: Option<usize>,
}
pub struct SearchEngineDocsTool {
    services: Arc<ToolServices>,
}
impl SearchEngineDocsTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for SearchEngineDocsTool {
    const NAME: &'static str = "search_engine_docs";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "搜索当前引擎快照文档，返回稳定 docId/sectionId 和受限片段。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"query":{"type":"string"},"max_chars":{"type":"integer","minimum":1,"maximum":4000}}),
            &["query"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let max = args.max_chars.unwrap_or(2000).min(4000);
        Ok(
            json!({"snapshotId":self.services.engine_snapshot.id,"hits":self.services.engine_snapshot.knowledge.search(&args.query,max)}),
        )
    }
}
