//! LLM 配置与 agent 构建。
//!
//! provider 选择与 agent 组装解耦：`build_agent` 只在 match 分支内做 provider 选择，
//! preamble / tools / temperature 都在分支之外，保证多个 provider 共用同一份组装逻辑。

use rig::tool::server::ToolServer;
use rig::tool::RegisteredTool;
use rig::{Agent, AgentBuilder};

const SYSTEM_PROMPT: &str =
    "你是配电网仿真智能体助手。根据用户请求，选择合适的工具完成模型检查或仿真任务。";

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
}

impl LlmConfig {
    /// 从环境变量读取，缺省时回落到 DeepSeek 默认值。
    pub fn from_env() -> Self {
        Self {
            provider: std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".to_string()),
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string()),
        }
    }
}

#[derive(Debug)]
pub enum LlmError {
    UnsupportedProvider(String),
    Provider(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::UnsupportedProvider(p) => write!(f, "provider '{p}' not implemented"),
            LlmError::Provider(e) => write!(f, "provider 初始化失败: {e}"),
        }
    }
}

impl std::error::Error for LlmError {}

/// 依据 `config.provider` 选择模型，然后在 provider 无关的公共逻辑里组装 agent。
pub fn build_agent(config: &LlmConfig, tools: Vec<RegisteredTool>) -> Result<Agent, LlmError> {
    // 1. provider 分支：只负责产出模型，其余配置一律不在这里做。
    let model = match config.provider.as_str() {
        "deepseek" => {
            let client = rig::providers::deepseek::from_env()
                .map_err(|e| LlmError::Provider(e.to_string()))?;
            client.chat(&config.model)
        }
        other => return Err(LlmError::UnsupportedProvider(other.to_string())),
    };

    // 2. 公共组装：preamble / tools / temperature 与 provider 无关。
    let mut server = ToolServer::new();
    for tool in tools {
        server = server.registered_tool(tool);
    }

    Ok(AgentBuilder::new(model)
        .preamble(SYSTEM_PROMPT)
        .temperature(0.3)
        .tool_server_handle(server.run())
        .build())
}
