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
pub struct DeepSeekAgentFactory {
    config: LlmConfig,
}

impl DeepSeekAgentFactory {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }
}

impl AgentFactory for DeepSeekAgentFactory {
    fn build(&self, request: AgentBuildRequest) -> Result<Agent, LlmError> {
        let model = match self.config.provider.as_str() {
            "deepseek" => {
                let client = rig::providers::deepseek::from_env()
                    .map_err(|error| LlmError::Provider(error.to_string()))?;
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
