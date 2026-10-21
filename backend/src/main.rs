//! 配电网仿真智能体后端。
//!
//! 工具用 rig 的 `#[rig_tool]` 宏定义，agent 由 `llm::build_agent` 依据
//! `LLM_PROVIDER` / `LLM_MODEL` 构建。

mod llm;

use rig::rig_tool;
use rig::tool::{RegisteredTool, ToolExecutionError};

use llm::LlmConfig;

/// 检查模型文件，返回检查结果（当前硬编码，后续改走适配器层）。
#[rig_tool(description = "检查一个配电网仿真模型文件，返回检查结果")]
fn inspect_model(path: String) -> Result<String, ToolExecutionError> {
    eprintln!("[tool] inspect_model 被调用，path = {path}");
    Ok("model ok".to_string())
}

/// 启动一次仿真，返回任务 ID（当前硬编码）。
#[rig_tool(description = "启动一次配电网仿真，返回任务 ID")]
fn run_simulation(model_path: String, duration_seconds: u64) -> Result<String, ToolExecutionError> {
    eprintln!(
        "[tool] run_simulation 被调用，model_path = {model_path}, duration_seconds = {duration_seconds}"
    );
    Ok("task-0001".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载 backend/.env（不存在也不报错，允许直接用 shell 里 export 的变量）
    dotenvy::dotenv().ok();

    let config = LlmConfig::from_env();
    eprintln!("[config] provider = {}, model = {}", config.provider, config.model);

    let tools = vec![
        RegisteredTool::from_tool(InspectModel),
        RegisteredTool::from_tool(RunSimulation),
    ];

    let agent = llm::build_agent(&config, tools)?;

    let response = agent.prompt("帮我检查一下这个模型").max_turns(8).await?;
    println!("agent 回复: {response}");

    Ok(())
}
