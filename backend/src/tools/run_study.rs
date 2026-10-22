use crate::application::RunStudyRequest;
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
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub model_version_id: i32,
    pub study_type: String,
    pub study_spec: Value,
    pub script: Option<String>,
}
pub struct RunStudyTool {
    services: Arc<ToolServices>,
}
impl RunStudyTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for RunStudyTool {
    const NAME: &'static str = "run_study";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "对 ready 模型版本执行潮流、短路或其他 tensoreval 仿真，并登记通用 Parquet 数据集。调用前必须加载 study-planning 与 rss-generation。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"modelVersionId":{"type":"integer"},"studyType":{"type":"string"},"studySpec":{"type":"object"},"script":{"type":["string","null"]}}),
            &["modelVersionId", "studyType", "studySpec"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("study-planning").await?;
        self.services.require_skill("rss-generation").await?;
        let mut refs = Vec::new();
        for id in ["study-planning", "rss-generation"] {
            let skill = self
                .services
                .agent_knowledge
                .skill(id)
                .ok_or_else(|| DomainToolError::new("skill_unavailable", id))?;
            refs.push(json!({"id":skill.metadata.id,"version":skill.metadata.version,"sha256":skill.metadata.sha256}));
        }
        let out = self
            .services
            .simulations
            .run(RunStudyRequest {
                principal: self.services.principal.clone(),
                session_id: self.services.session_id,
                model_version_id: args.model_version_id,
                study_type: args.study_type,
                study_spec: args.study_spec,
                script: args.script,
                skill_refs: refs,
            })
            .await?;
        Ok(
            json!({"taskId":out.task.id.to_string(),"status":out.task.status,"datasets":out.datasets.into_iter().map(|d|json!({"datasetId":d.id.to_string(),"name":d.result_name,"rowCount":d.row_count})).collect::<Vec<_>>()}),
        )
    }
}
