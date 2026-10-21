//! 配电网仿真智能体后端。
//!
//! 工具用 rig 的 `#[rig_tool]` 宏定义，agent 由 `llm::build_agent` 依据
//! `LLM_PROVIDER` / `LLM_MODEL` 构建。对外暴露最小 HTTP 接口：
//! `GET /health` 健康检查，`POST /api/chat` 对话。

mod db;
mod llm;

use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};
use rig::rig_tool;
use rig::tool::{RegisteredTool, ToolExecutionError};
use serde::Deserialize;

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

#[derive(Deserialize)]
struct ChatRequest {
    message: String,
}

async fn health() -> &'static str {
    "ok"
}

async fn chat(Json(req): Json<ChatRequest>) -> (StatusCode, Json<serde_json::Value>) {
    // 每个请求按当前环境变量构建 agent，便于 provider 切换与错误返回。
    let config = LlmConfig::from_env();
    let tools = vec![
        RegisteredTool::from_tool(InspectModel),
        RegisteredTool::from_tool(RunSimulation),
    ];

    let agent = match llm::build_agent(&config, tools) {
        Ok(agent) => agent,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            );
        }
    };

    match agent.prompt(req.message).max_turns(8).await {
        Ok(reply) => (
            StatusCode::OK,
            Json(serde_json::json!({ "reply": reply.output() })),
        ),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": err.to_string() })),
        ),
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        // 用 Display 打印可读错误（例如 provider 未接入 / 不支持的提示），并保留非零退出码。
        eprintln!("错误: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 加载 backend/.env（不存在也不报错，允许直接用 shell 里 export 的变量）
    dotenvy::dotenv().ok();

    let config = LlmConfig::from_env();
    eprintln!("[config] provider = {}, model = {}", config.provider, config.model);

    // 建立 SQLite 连接池并应用嵌入式迁移。
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://backend.db".to_string());
    let pool = db::establish_pool(&database_url)?;
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn)?;
    }
    eprintln!("[db] ready at {database_url}");

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/chat", post(chat))
        .with_state(pool);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[http] listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
