use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::{LockedStudyContract, WorkflowInputSnapshot};

use super::{AgentBuildRequest, AgentFactory, DeepSeekAgentFactory, LlmConfig};

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
}

#[derive(Clone)]
pub struct DeepSeekScriptAuthor {
    factory: Arc<dyn AgentFactory>,
    model_retry_max: usize,
}

impl DeepSeekScriptAuthor {
    pub fn new(config: LlmConfig, model_retry_max: usize) -> Self {
        Self {
            factory: Arc::new(DeepSeekAgentFactory::new(config)),
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
                    system_prompt: format!(
                        "你是受控 RSS 脚本作者。根据冻结合同{purpose}。只输出完整脚本，不要解释，不要使用 import_dss 或其他导入模型的方法。"
                    ),
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
                Ok(script) => return Ok(script.output().to_string()),
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
impl ScriptAuthor for DeepSeekScriptAuthor {
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
