use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::{LockedStudyContract, WorkflowInputSnapshot};

use super::script_author::normalize_script_output;
use super::{AgentBuildRequest, AgentFactory};

const FAILURE_ANALYSIS_RULES: &str = r#"你是配电网仿真失败诊断助手。只依据请求中的已脱敏日志、受控 diagnostic、冻结合同和脚本给出面向用户的诊断摘要。
只输出一个 JSON 对象，不要输出 Markdown 或思维过程。必须区分已确认事实、可能原因、未知信息和建议动作；不得把推断写成事实。
facts 和 possibleCauses 的每一项都必须引用具体的 stderr 日志片段，evidence 以 `stderr:` 开头。不得复原或猜测已脱敏路径、密钥或命令。"#;

const MAX_ANALYSIS_ITEMS: usize = 5;
const MAX_ANALYSIS_SUMMARY_CHARS: usize = 500;
const MAX_ANALYSIS_ITEM_CHARS: usize = 800;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureAnalysisRequest {
    pub current_script: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sanitized_stdout: Option<String>,
    pub sanitized_stderr: String,
    pub diagnostic: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_contract: Option<LockedStudyContract>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<WorkflowInputSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureAnalysisEvidence {
    pub statement: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureAnalysis {
    pub summary: String,
    pub facts: Vec<FailureAnalysisEvidence>,
    pub possible_causes: Vec<FailureAnalysisEvidence>,
    pub unknowns: Vec<String>,
    pub recommendations: Vec<String>,
}

pub fn validate_failure_analysis(
    analysis: &FailureAnalysis,
    sanitized_stderr: &str,
) -> Result<(), FailureAnalysisError> {
    validate_analysis_text(&analysis.summary, MAX_ANALYSIS_SUMMARY_CHARS)?;
    for items in [&analysis.facts, &analysis.possible_causes] {
        if items.len() > MAX_ANALYSIS_ITEMS {
            return Err(FailureAnalysisError::provider("失败诊断条目过多"));
        }
        for item in items {
            validate_analysis_text(&item.statement, MAX_ANALYSIS_ITEM_CHARS)?;
            validate_analysis_text(&item.evidence, MAX_ANALYSIS_ITEM_CHARS)?;
            if !item.evidence.starts_with("stderr:") {
                return Err(FailureAnalysisError::provider("失败诊断缺少日志引用"));
            }
            let excerpt = item.evidence["stderr:".len()..].trim();
            if excerpt.is_empty() || !sanitized_stderr.contains(excerpt) {
                return Err(FailureAnalysisError::provider("失败诊断引用不在日志中"));
            }
        }
    }
    for items in [&analysis.unknowns, &analysis.recommendations] {
        if items.len() > MAX_ANALYSIS_ITEMS {
            return Err(FailureAnalysisError::provider("失败诊断条目过多"));
        }
        for item in items {
            validate_analysis_text(item, MAX_ANALYSIS_ITEM_CHARS)?;
        }
    }
    Ok(())
}

fn parse_failure_analysis(
    output: &str,
    sanitized_stderr: &str,
) -> Result<FailureAnalysis, FailureAnalysisError> {
    let normalized = normalize_script_output(output);
    let analysis: FailureAnalysis = serde_json::from_str(&normalized)
        .map_err(|_| FailureAnalysisError::provider("失败诊断模型返回格式无效"))?;
    validate_failure_analysis(&analysis, sanitized_stderr)?;
    Ok(analysis)
}

fn validate_analysis_text(value: &str, max_chars: usize) -> Result<(), FailureAnalysisError> {
    if value.trim().is_empty() || value.chars().count() > max_chars {
        return Err(FailureAnalysisError::provider("失败诊断文本长度无效"));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct FailureAnalysisError {
    message: String,
}

impl FailureAnalysisError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for FailureAnalysisError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for FailureAnalysisError {}

#[async_trait]
pub trait FailureAnalyzer: Send + Sync {
    async fn analyze_failure(
        &self,
        request: FailureAnalysisRequest,
    ) -> Result<FailureAnalysis, FailureAnalysisError>;
}

#[derive(Clone)]
pub struct LlmFailureAnalyzer {
    factory: Arc<dyn AgentFactory>,
    model_retry_max: usize,
}

impl LlmFailureAnalyzer {
    pub fn from_factory(factory: Arc<dyn AgentFactory>, model_retry_max: usize) -> Self {
        Self {
            factory,
            model_retry_max,
        }
    }
}

#[async_trait]
impl FailureAnalyzer for LlmFailureAnalyzer {
    async fn analyze_failure(
        &self,
        request: FailureAnalysisRequest,
    ) -> Result<FailureAnalysis, FailureAnalysisError> {
        let payload = serde_json::to_string(&request)
            .map_err(|error| FailureAnalysisError::provider(error.to_string()))?;
        let mut last_error = None;
        for attempt in 0..=self.model_retry_max {
            let agent = match self.factory.build(AgentBuildRequest {
                tools: Vec::new(),
                system_prompt: FAILURE_ANALYSIS_RULES.into(),
            }) {
                Ok(agent) => agent,
                Err(error) => {
                    last_error = Some(error.to_string());
                    if attempt < self.model_retry_max {
                        tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await;
                        continue;
                    }
                    break;
                }
            };
            match agent.prompt(payload.clone()).await {
                Ok(response) => {
                    match parse_failure_analysis(&response.output(), &request.sanitized_stderr) {
                        Ok(analysis) => return Ok(analysis),
                        Err(error) => last_error = Some(error.to_string()),
                    }
                }
                Err(error) => last_error = Some(error.to_string()),
            }
            if attempt < self.model_retry_max {
                tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await;
            }
        }
        Err(FailureAnalysisError::provider(
            last_error.unwrap_or_else(|| "失败诊断模型调用失败".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        FailureAnalysis, FailureAnalysisEvidence, FailureAnalysisRequest, parse_failure_analysis,
        validate_failure_analysis,
    };

    fn valid_analysis(evidence: &str) -> FailureAnalysis {
        FailureAnalysis {
            summary: "引擎拒绝了当前脚本".into(),
            facts: vec![FailureAnalysisEvidence {
                statement: "引擎报告解析失败".into(),
                evidence: evidence.into(),
            }],
            possible_causes: Vec::new(),
            unknowns: vec!["具体语法位置".into()],
            recommendations: vec!["核对脚本语法".into()],
        }
    }

    #[test]
    fn accepts_optional_workflow_context_for_ordinary_runs() {
        let request = FailureAnalysisRequest {
            current_script: "result = solve(table);".into(),
            sanitized_stdout: Some("engine banner".into()),
            sanitized_stderr: "parse failed at line 1".into(),
            diagnostic: Some(json!({"exitCode": 7})),
            locked_contract: None,
            inputs: None,
        };

        assert!(
            validate_failure_analysis(
                &valid_analysis("stderr: parse failed"),
                &request.sanitized_stderr,
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_stdout_text_cited_as_stderr_evidence() {
        assert!(
            validate_failure_analysis(
                &valid_analysis("stderr: engine banner"),
                "parse failed at line 1",
            )
            .is_err()
        );
    }

    #[test]
    fn parses_and_bounds_user_facing_failure_analysis() {
        let analysis = parse_failure_analysis(
            r#"{
                "summary": "潮流算法拒绝了当前拓扑",
                "facts": [{"statement": "引擎报告拓扑不受支持", "evidence": "stderr: network topology is unsupported"}],
                "possibleCauses": [{"statement": "所选算法与当前网络拓扑不兼容", "evidence": "stderr: selected power-flow method"}],
                "unknowns": ["当前拓扑的具体结构"],
                "recommendations": ["核对该算法支持的拓扑条件"]
            }"#,
            "network topology is unsupported; selected power-flow method",
        )
        .expect("valid bounded analysis");

        assert_eq!(analysis.facts.len(), 1);
        assert_eq!(analysis.possible_causes.len(), 1);
    }

    #[test]
    fn rejects_unbounded_or_non_json_failure_analysis() {
        assert!(parse_failure_analysis("not json", "").is_err());
        let oversized = format!(
            "{{\"summary\":\"{}\",\"facts\":[],\"possibleCauses\":[],\"unknowns\":[],\"recommendations\":[]}}",
            "x".repeat(501)
        );
        assert!(parse_failure_analysis(&oversized, "").is_err());
    }
}
