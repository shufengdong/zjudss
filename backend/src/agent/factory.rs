use rig::tool::RegisteredTool;
use rig::tool::server::ToolServer;
use rig::{Agent, AgentBuilder};

use super::{LlmConfig, LlmError};

pub struct AgentBuildRequest {
    pub tools: Vec<RegisteredTool>,
    pub system_prompt: String,
}

pub trait AgentFactory: Send + Sync {
    fn build(&self, request: AgentBuildRequest) -> Result<Agent, LlmError>;
}

#[derive(Debug, Clone)]
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
