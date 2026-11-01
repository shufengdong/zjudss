use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::{LockedStudyContract, WorkflowInputSnapshot};

use super::{AgentBuildRequest, AgentFactory, LlmAgentFactory, LlmConfig};

const SCRIPT_AUTHOR_RULES: &str = r#"你是受控 RSS 脚本作者。请求 JSON 中 locked/lockedContract 是不可改变的冻结合同。
运行环境已经按 locked.inputAliases 注入全部 DataFrame，包括 Struct DataFrame；不得读取、导入、构造、重新赋值或重命名这些变量。
完整脚本只写一条直接结果绑定：左侧严格使用 locked.results 中的 name，右侧严格调用一次 locked.function，实参严格按 locked.inputAliases 的原顺序逐项引用。
不得使用 read_table、read、read_csv、read_parquet、import_dss、struct、print 或其他辅助函数，不得追加裸变量表达式。
只输出纯 RSS 源码，不要解释，不要使用 Markdown 代码围栏。修正脚本时必须按 diagnostic 和 sanitizedStderr 修复 currentScript，不得原样返回已被拒绝的脚本。"#;

const FAILURE_ANALYSIS_RULES: &str = r#"你是配电网仿真失败诊断助手。只依据请求中的已脱敏日志、受控 diagnostic、冻结合同和脚本给出面向用户的诊断摘要。
只输出一个 JSON 对象，不要输出 Markdown 或思维过程。必须区分已确认事实、可能原因、未知信息和建议动作；不得把推断写成事实。
facts 和 possibleCauses 的每一项都必须引用具体的 stderr 日志片段，evidence 以 `stderr:` 开头。不得复原或猜测已脱敏路径、密钥或命令。"#;

const MAX_ANALYSIS_ITEMS: usize = 5;
const MAX_ANALYSIS_SUMMARY_CHARS: usize = 500;
const MAX_ANALYSIS_ITEM_CHARS: usize = 800;

fn normalize_script_output(output: &str) -> String {
    let trimmed = output.trim();
    let Some(after_fence) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    let Some(first_line_end) = after_fence.find('\n') else {
        return trimmed.to_string();
    };
    let fenced = &after_fence[first_line_end + 1..];
    let Some(last_fence) = fenced.rfind("```") else {
        return trimmed.to_string();
    };
    fenced[..last_fence].trim().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineDocRef {
    pub id: String,
    pub content_hash: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptGenerationRequest {
    pub locked: LockedStudyContract,
    pub inputs: WorkflowInputSnapshot,
    pub engine_docs: Vec<EngineDocRef>,
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRepairRequest {
    pub current_script: String,
    pub sanitized_stderr: String,
    pub diagnostic: Option<Value>,
    pub locked_contract: LockedStudyContract,
    pub inputs: WorkflowInputSnapshot,
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureAnalysisRequest {
    pub current_script: String,
    pub sanitized_stderr: String,
    pub diagnostic: Option<Value>,
    pub locked_contract: LockedStudyContract,
    pub inputs: WorkflowInputSnapshot,
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

fn parse_failure_analysis(
    output: &str,
    sanitized_stderr: &str,
) -> Result<FailureAnalysis, ScriptAuthorError> {
    let normalized = normalize_script_output(output);
    let analysis: FailureAnalysis = serde_json::from_str(&normalized)
        .map_err(|_| ScriptAuthorError::provider("失败诊断模型返回格式无效"))?;
    validate_analysis_text(&analysis.summary, MAX_ANALYSIS_SUMMARY_CHARS)?;
    for items in [&analysis.facts, &analysis.possible_causes] {
        if items.len() > MAX_ANALYSIS_ITEMS {
            return Err(ScriptAuthorError::provider("失败诊断条目过多"));
        }
        for item in items {
            validate_analysis_text(&item.statement, MAX_ANALYSIS_ITEM_CHARS)?;
            validate_analysis_text(&item.evidence, MAX_ANALYSIS_ITEM_CHARS)?;
            if !item.evidence.starts_with("stderr:") {
                return Err(ScriptAuthorError::provider("失败诊断缺少日志引用"));
            }
            let excerpt = item.evidence["stderr:".len()..].trim();
            if excerpt.is_empty() || !sanitized_stderr.contains(excerpt) {
                return Err(ScriptAuthorError::provider("失败诊断引用不在日志中"));
            }
        }
    }
    for items in [&analysis.unknowns, &analysis.recommendations] {
        if items.len() > MAX_ANALYSIS_ITEMS {
            return Err(ScriptAuthorError::provider("失败诊断条目过多"));
        }
        for item in items {
            validate_analysis_text(item, MAX_ANALYSIS_ITEM_CHARS)?;
        }
    }
    Ok(analysis)
}

fn validate_analysis_text(value: &str, max_chars: usize) -> Result<(), ScriptAuthorError> {
    if value.trim().is_empty() || value.chars().count() > max_chars {
        return Err(ScriptAuthorError::provider("失败诊断文本长度无效"));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ScriptAuthorError {
    message: String,
}

impl ScriptAuthorError {
    pub fn provider(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScriptAuthorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for ScriptAuthorError {}

#[async_trait]
pub trait ScriptAuthor: Send + Sync {
    async fn generate(&self, request: ScriptGenerationRequest)
    -> Result<String, ScriptAuthorError>;

    async fn repair(&self, request: ScriptRepairRequest) -> Result<String, ScriptAuthorError>;

    async fn analyze_failure(
        &self,
        request: FailureAnalysisRequest,
    ) -> Result<FailureAnalysis, ScriptAuthorError>;
}

#[derive(Clone)]
pub struct LlmScriptAuthor {
    factory: Arc<dyn AgentFactory>,
    model_retry_max: usize,
}

impl LlmScriptAuthor {
    pub fn new(config: LlmConfig, model_retry_max: usize) -> Self {
        Self {
            factory: Arc::new(LlmAgentFactory::new(config)),
            model_retry_max,
        }
    }

    pub fn from_factory(factory: Arc<dyn AgentFactory>, model_retry_max: usize) -> Self {
        Self {
            factory,
            model_retry_max,
        }
    }

    async fn complete<T: Serialize>(
        &self,
        purpose: &str,
        request: &T,
    ) -> Result<String, ScriptAuthorError> {
        let payload = serde_json::to_string(request)
            .map_err(|error| ScriptAuthorError::provider(error.to_string()))?;
        let mut last_error = None;
        for attempt in 0..=self.model_retry_max {
            let agent = match self.factory.build(AgentBuildRequest {
                tools: Vec::new(),
                system_prompt: format!("根据冻结合同{purpose}。\n{SCRIPT_AUTHOR_RULES}"),
            }) {
                Ok(agent) => agent,
                Err(error) => {
                    if last_error.is_none() {
                        last_error = Some(error.to_string());
                    }
                    if attempt < self.model_retry_max {
                        tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await;
                        continue;
                    }
                    break;
                }
            };
            match agent.prompt(payload.clone()).await {
                Ok(script) => return Ok(normalize_script_output(&script.output())),
                Err(error) => {
                    last_error = Some(error.to_string());
                    if attempt < self.model_retry_max {
                        tokio::time::sleep(Duration::from_millis(50 * (attempt as u64 + 1))).await;
                    }
                }
            }
        }
        Err(ScriptAuthorError::provider(
            last_error.unwrap_or_else(|| "脚本模型调用失败".into()),
        ))
    }

    async fn complete_failure_analysis(
        &self,
        request: &FailureAnalysisRequest,
    ) -> Result<FailureAnalysis, ScriptAuthorError> {
        let payload = serde_json::to_string(request)
            .map_err(|error| ScriptAuthorError::provider(error.to_string()))?;
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
        Err(ScriptAuthorError::provider(
            last_error.unwrap_or_else(|| "失败诊断模型调用失败".into()),
        ))
    }
}

#[async_trait]
impl ScriptAuthor for LlmScriptAuthor {
    async fn generate(
        &self,
        request: ScriptGenerationRequest,
    ) -> Result<String, ScriptAuthorError> {
        self.complete("生成脚本", &request).await
    }

    async fn repair(&self, request: ScriptRepairRequest) -> Result<String, ScriptAuthorError> {
        self.complete("修正脚本", &request).await
    }

    async fn analyze_failure(
        &self,
        request: FailureAnalysisRequest,
    ) -> Result<FailureAnalysis, ScriptAuthorError> {
        self.complete_failure_analysis(&request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_contract_forbids_rebinding_and_markdown() {
        for required in [
            "不得读取、导入、构造、重新赋值或重命名",
            "locked.inputAliases",
            "locked.results",
            "locked.function",
            "不得使用 read_table",
            "不要使用 Markdown 代码围栏",
            "不得原样返回",
        ] {
            assert!(SCRIPT_AUTHOR_RULES.contains(required), "missing {required}");
        }
    }

    #[test]
    fn normalizes_one_markdown_fence_from_provider_output() {
        assert_eq!(
            normalize_script_output("```rss\n[result] = solve(table, para);\n```"),
            "[result] = solve(table, para);"
        );
        assert_eq!(
            normalize_script_output("[result] = solve(table, para);\n"),
            "[result] = solve(table, para);"
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
        assert!(analysis.facts[0].evidence.starts_with("stderr:"));
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

    #[test]
    fn rejects_failure_analysis_evidence_not_present_in_stderr() {
        let hallucinated = r#"{
            "summary": "诊断摘要",
            "facts": [{"statement": "日志包含不存在的结论", "evidence": "stderr: invented evidence"}],
            "possibleCauses": [],
            "unknowns": ["真实原因"],
            "recommendations": ["检查原始日志"]
        }"#;

        assert!(parse_failure_analysis(hallucinated, "actual engine output").is_err());
    }
}
