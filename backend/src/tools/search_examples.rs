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
        "按名称或分析类型搜索当前资源快照中的内置案例，返回稳定 exampleId。".into()
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
        let query = args.query.unwrap_or_default().to_lowercase();
        let values: Vec<_> = self
            .services
            .engine_snapshot
            .examples
            .iter()
            .filter(|e| {
                (query.is_empty()
                    || e.title.to_lowercase().contains(&query)
                    || e.id.to_lowercase().contains(&query))
                    && args.study_type.as_ref().is_none_or(|study_type| {
                        e.manifest
                            .as_ref()
                            .is_some_and(|manifest| study_type == &manifest.study.kind)
                    })
            })
            .take(50)
            .collect();
        Ok(json!({"snapshotId":self.services.engine_snapshot.id,"examples":values}))
    }
}
