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
    pub doc_id: String,
    pub section_id: String,
    #[serde(default)]
    pub max_chars: Option<usize>,
}
pub struct ReadEngineDocSectionTool {
    services: Arc<ToolServices>,
}
impl ReadEngineDocSectionTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for ReadEngineDocSectionTool {
    const NAME: &'static str = "read_engine_doc_section";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按稳定 docId/sectionId 读取当前引擎文档的受限片段；不接受文件路径。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"doc_id":{"type":"string"},"section_id":{"type":"string"},"max_chars":{"type":"integer","minimum":1,"maximum":8000}}),
            &["doc_id", "section_id"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let section = self
            .services
            .engine_snapshot
            .knowledge
            .read(&args.section_id, args.max_chars.unwrap_or(4000).min(8000))
            .filter(|s| s.doc_id == args.doc_id)
            .ok_or_else(|| DomainToolError::new("doc_section_not_found", "未知 docId/sectionId"))?;
        Ok(json!(section))
    }
}
