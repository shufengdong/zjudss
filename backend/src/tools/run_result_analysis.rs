use crate::application::{AnalysisInput, AnalysisInputSource, AnalysisRequest};
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};
use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputArg {
    dataset_id: Option<i32>,
    resource_id: Option<String>,
    alias: String,
}
#[derive(Deserialize)]
pub struct ViewArg {
    name: String,
    spec: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    intent: Value,
    script: String,
    inputs: Vec<InputArg>,
    expected_outputs: Vec<String>,
    #[serde(default)]
    initial_views: HashMap<String, Vec<ViewArg>>,
}
pub struct RunResultAnalysisTool {
    services: Arc<ToolServices>,
}
impl RunResultAnalysisTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for RunResultAnalysisTool {
    const NAME: &'static str = "run_result_analysis";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "根据用户自然语言问题运行受控 tensoreval 分析脚本，从已有数据集产生新的 Parquet DataFrame。调用前必须加载 result-analysis。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"intent":{"type":"object"},"script":{"type":"string"},"inputs":{"type":"array","items":{"type":"object","properties":{"datasetId":{"type":["integer","null"]},"resourceId":{"type":["string","null"]},"alias":{"type":"string"}},"required":["alias"],"oneOf":[{"required":["datasetId"]},{"required":["resourceId"]}]}},"expectedOutputs":{"type":"array","items":{"type":"string"}},"initialViews":{"type":"object"}}),
            &["intent", "script", "inputs", "expectedOutputs"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("result-analysis").await?;
        let skill = self
            .services
            .agent_knowledge
            .skill("result-analysis")
            .unwrap();
        let views = args
            .initial_views
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().map(|x| (x.name, x.spec)).collect()))
            .collect();
        let inputs = args
            .inputs
            .into_iter()
            .map(|value| {
                let source = match (value.dataset_id, value.resource_id) {
                    (Some(id), None) => Ok(AnalysisInputSource::DatasetId(id)),
                    (None, Some(id)) => Ok(AnalysisInputSource::DataResourceId(id)),
                    _ => Err(DomainToolError::new(
                        "analysis_inputs_invalid",
                        "每个输入必须且只能提供 datasetId 或 resourceId",
                    )),
                }?;
                Ok(AnalysisInput {
                    source,
                    alias: value.alias,
                })
            })
            .collect::<Result<Vec<_>, DomainToolError>>()?;
        let out=self.services.analysis.run(AnalysisRequest{principal:self.services.principal.clone(),session_id:self.services.session_id,request_message_id:Some(self.services.request_message_id),intent:args.intent,script:args.script,inputs,expected_outputs:args.expected_outputs,initial_views:views,skill_refs:vec![json!({"id":skill.metadata.id,"version":skill.metadata.version,"sha256":skill.metadata.sha256})]}).await?;
        Ok(
            json!({"analysisRunId":out.run.id.to_string(),"status":out.run.status,"datasets":out.datasets.into_iter().map(|d|json!({"datasetId":d.id.to_string(),"name":d.result_name})).collect::<Vec<_>>() }),
        )
    }
}
