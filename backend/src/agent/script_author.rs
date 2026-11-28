//! RSS 脚本生成与修复的隔离 LLM 边界。
//!
//! 输入只包含冻结合同和按需引擎文档，输出在交给适配器前被规范化为脚本文本。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::{LockedStudyContract, WorkflowInputSnapshot};

use super::{AgentBuildRequest, AgentFactory, LlmAgentFactory, LlmConfig};

const SCRIPT_AUTHOR_RULES: &str = r#"你是受控 RSS 脚本作者。请求 JSON 中 locked/lockedContract 是不可改变的冻结合同。
运行环境已经按 locked.inputAliases 注入全部 DataFrame，包括 Struct DataFrame；不得读取、导入、构造、重新赋值或重命名这些变量。
请求 JSON 中 argumentSignature 是后端根据 locked.inputAliases 精确拼接的实参列表原文，resultBinding 是后端根据 locked.results 精确拼接的左值原文。你必须原样复制这两个字段，不得改动、排序、增删任何 alias 或 result 名。
完整脚本只写一条直接结果绑定：左侧原样复制 resultBinding，右侧写 locked.function 并在括号内原样复制 argumentSignature。
单结果左值形如 "result_name"，多结果左值形如 "[name1, name2, name3]"（方括号已包含在 resultBinding 里）。
不得使用 read_table、read、read_csv、read_parquet、import_dss、struct、print 或其他辅助函数，不得追加裸变量表达式。
不要添加分号结尾。只输出纯 RSS 源码，不要解释，不要使用 Markdown 代码围栏。修正脚本时必须按 diagnostic 和 sanitizedStderr 修复 currentScript，不得原样返回已被拒绝的脚本。"#;

pub(super) fn normalize_script_output(output: &str) -> String {
    let cleaned = output
        .trim_start_matches('\u{feff}')
        .trim_start_matches('\u{200b}')
        .trim_start_matches('\u{200c}')
        .trim_start_matches('\u{200d}')
        .trim();
    let body = if let Some(after_fence) = cleaned.strip_prefix("```") {
        match after_fence.find('\n') {
            Some(end) => {
                let fenced = &after_fence[end + 1..];
                match fenced.rfind("```") {
                    Some(last) => fenced[..last].trim().to_string(),
                    None => fenced.trim().to_string(),
                }
            }
            None => cleaned.to_string(),
        }
    } else {
        cleaned.to_string()
    };
    body.trim_end_matches(';').trim().to_string()
}

pub fn build_argument_signature(aliases: &[String]) -> String {
    aliases.iter().cloned().collect::<Vec<_>>().join(", ")
}

pub fn build_result_binding(_result_names: &[String]) -> String {
    "result".to_string()
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
    pub argument_signature: String,
    pub result_binding: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRepairRequest {
    pub current_script: String,
    pub sanitized_stderr: String,
    pub diagnostic: Option<Value>,
    pub locked_contract: LockedStudyContract,
    pub inputs: WorkflowInputSnapshot,
    pub engine_docs: Vec<EngineDocRef>,
    pub instruction: String,
    pub argument_signature: String,
    pub result_binding: String,
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
/// 依据冻结合同生成或修复 RSS 的隔离接口。
pub trait ScriptAuthor: Send + Sync {
    async fn generate(&self, request: ScriptGenerationRequest)
    -> Result<String, ScriptAuthorError>;

    async fn repair(&self, request: ScriptRepairRequest) -> Result<String, ScriptAuthorError>;
}

#[derive(Clone)]
/// 使用无工具 Agent 生成纯 RSS 文本的生产实现。
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_contract_forbids_rebinding_and_markdown() {
        for required in [
            "不得读取、导入、构造、重新赋值或重命名",
            "argumentSignature",
            "resultBinding",
            "locked.function",
            "原样复制这两个字段",
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
            "[result] = solve(table, para)"
        );
        assert_eq!(
            normalize_script_output("[result] = solve(table, para);\n"),
            "[result] = solve(table, para)"
        );
        assert_eq!(
            normalize_script_output("[result] = solve(table, para)\n"),
            "[result] = solve(table, para)"
        );
    }

    #[test]
    fn builds_argument_signature_and_result_binding() {
        let aliases = vec![
            "bus".into(),
            "line".into(),
            "monitor_input".into(),
        ];
        assert_eq!(
            build_argument_signature(&aliases),
            "bus, line, monitor_input"
        );
        assert_eq!(build_result_binding(&["result".into()]), "result");
        assert_eq!(
            build_result_binding(&["execution.bus_voltage".into(), "fault_study.fault_result".into()]),
            "result"
        );
    }
}