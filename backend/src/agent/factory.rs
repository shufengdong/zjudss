//! 按请求构造 LLM Agent，并只注册调用方依据 manifest 选择的工具集合。

use rig::tool::RegisteredTool;
use rig::tool::server::ToolServer;
use rig::{Agent, AgentBuilder};

use super::{LlmConfig, LlmError};

/// 单次 Agent 构造所需的已筛选工具与完整系统 Prompt。
pub struct AgentBuildRequest {
    pub tools: Vec<RegisteredTool>,
    pub system_prompt: String,
}

/// 按请求创建相互隔离的 Agent 实例。
pub trait AgentFactory: Send + Sync {
    fn build(&self, request: AgentBuildRequest) -> Result<Agent, LlmError>;
}

#[derive(Debug, Clone)]
/// 根据服务配置连接 LLM provider 的生产 Agent 工厂。
pub struct LlmAgentFactory {
    config: LlmConfig,
}

impl LlmAgentFactory {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }
}

fn required_env(name: &'static str) -> Result<String, LlmError> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or(LlmError::MissingConfiguration(name))
}

impl AgentFactory for LlmAgentFactory {
    fn build(&self, request: AgentBuildRequest) -> Result<Agent, LlmError> {
        let model = match self.config.provider.as_str() {
            "deepseek" => {
                let client = rig::providers::deepseek::from_env()
                    .map_err(|error| LlmError::Provider(error.to_string()))?;
                client.chat(&self.config.model)
            }
            "bailian" => {
                let api_key = required_env("BAILIAN_API_KEY")?;
                let base_url = required_env("BAILIAN_BASE_URL")?;
                let client = rig::providers::openai::OpenAIConfig::new(api_key)
                    .with_base_url(base_url)
                    .client();
                client.chat(&self.config.model)
            }
            "openai" => return Err(LlmError::OpenAiNotImplemented),
            other => return Err(LlmError::UnsupportedProvider(other.to_string())),
        };

        let mut server = ToolServer::new();
        for tool in request.tools {
            server = server.registered_tool(tool);
        }
        Ok(AgentBuilder::new(model)
            .preamble(request.system_prompt)
            .temperature(0.3)
            .tool_server_handle(server.run())
            .build())
    }
}
