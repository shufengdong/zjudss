use crate::application::{AnalysisInput, AnalysisInputSource, AnalysisRequest};
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};
use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_INTENT_CHARS: usize = 1_000;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputArg {
    dataset_id: Option<i32>,
    resource_id: Option<String>,
    alias: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Args {
    intent: String,
    script: String,
    inputs: Vec<InputArg>,
    expected_outputs: Vec<String>,
}

fn normalize_intent(intent: String) -> Result<String, DomainToolError> {
    let intent = intent.trim().to_owned();
    if intent.is_empty() || intent.chars().count() > MAX_INTENT_CHARS {
        return Err(DomainToolError::new(
            "analysis_intent_invalid",
            "分析目的必须为 1 到 1000 个字符",
        ));
    }
    Ok(intent)
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
            json!({"intent":{"type":"string","minLength":1,"maxLength":MAX_INTENT_CHARS,"description":"简洁复述用户要求执行的结果分析目标，仅用于审计"},"script":{"type":"string"},"inputs":{"type":"array","items":{"type":"object","properties":{"datasetId":{"type":["integer","null"]},"resourceId":{"type":["string","null"]},"alias":{"type":"string"}},"required":["alias"],"oneOf":[{"required":["datasetId"]},{"required":["resourceId"]}]}},"expectedOutputs":{"type":"array","items":{"type":"string"}}}),
            &["intent", "script", "inputs", "expectedOutputs"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let intent = normalize_intent(args.intent)?;
        self.services.require_skill("result-analysis").await?;
        let skill = self
            .services
            .agent_knowledge
            .skill("result-analysis")
            .unwrap();
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
        let out=self.services.analysis.run(AnalysisRequest{principal:self.services.principal.clone(),session_id:self.services.session_id,request_message_id:Some(self.services.request_message_id),intent:Value::String(intent),script:args.script,inputs,expected_outputs:args.expected_outputs,skill_refs:vec![json!({"id":skill.metadata.id,"version":skill.metadata.version,"sha256":skill.metadata.sha256})]}).await?;
        Ok(
            json!({"analysisRunId":out.run.id.to_string(),"status":out.run.status,"datasets":out.datasets.into_iter().map(|d|json!({"datasetId":d.id.to_string(),"name":d.result_name})).collect::<Vec<_>>() }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_intent;

    #[test]
    fn analysis_intent_trims_and_preserves_one_thousand_unicode_characters() {
        let value = format!("  {}  ", "压".repeat(1_000));

        let normalized = normalize_intent(value).expect("bounded intent must be accepted");

        assert_eq!(normalized.chars().count(), 1_000);
        assert!(!normalized.starts_with(' '));
        assert!(!normalized.ends_with(' '));
    }

    #[test]
    fn analysis_intent_rejects_whitespace_only_text() {
        let error = normalize_intent(" \n\t ".into()).expect_err("blank intent must fail");

        assert_eq!(error.code, "analysis_intent_invalid");
    }

    #[test]
    fn analysis_intent_rejects_more_than_one_thousand_unicode_characters() {
        let error = normalize_intent("压".repeat(1_001)).expect_err("oversized intent must fail");

        assert_eq!(error.code, "analysis_intent_invalid");
    }
}
