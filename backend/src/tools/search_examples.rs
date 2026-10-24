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
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub study_type: Option<String>,
}
pub struct SearchExamplesTool {
    services: Arc<ToolServices>,
}
impl SearchExamplesTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for SearchExamplesTool {
    const NAME: &'static str = "search_examples";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按名称、目录路径或 manifest 分析元数据搜索内置案例，返回稳定 exampleId、目录路径和验证状态。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"query":{"type":"string"},"study_type":{"type":"string"}}),
            &[],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let query = args.query.unwrap_or_default();
        let values: Vec<_> = self
            .services
            .engine_snapshot
            .examples
            .iter()
            .filter(|example| example.matches_catalog_query(&query, args.study_type.as_deref()))
            .take(50)
            .map(example_summary)
            .collect();
        Ok(json!({"snapshotId":self.services.engine_snapshot.id,"examples":values}))
    }
}

fn example_summary(example: &crate::engine_resources::EngineExample) -> Value {
    let study = example.manifest.as_ref().map(|manifest| &manifest.study);
    json!({
        "exampleId": example.id,
        "title": example.title,
        "categoryPath": example.category_path,
        "bundlePath": example.bundle_path,
        "studyType": study.map(|study| study.kind.as_str()),
        "solver": study.and_then(|study| study.solver.as_deref()),
        "compatibility": study.map(|study| study.compatibility.as_str()),
        "results": study.map_or_else(Vec::new, |study| study.results.clone()),
        "sourceEntry": example
            .manifest
            .as_ref()
            .map(|manifest| manifest.source.entry.as_str()),
        "importable": example.importable,
        "diagnostics": example.diagnostics,
    })
}
