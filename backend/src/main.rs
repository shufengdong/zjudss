//! 配电网仿真智能体后端。
//!
//! 分层：`api`（HTTP/SSE）→ `application`（编排/事件）→ `tools`/`adapters`（内核边界）
//! → `db`（持久）。入口只负责装配 `AppState`、应用迁移、挂路由。
//!
//! 当前进度（`docs/architecture.md` §12）：第 1~3 步已落地；第 4 步（真实引擎子进程
//! `adapters::ProcessEngine` + 任务状态机 + 限流）已落地，引擎由 `TENSOREVAL_BIN` 选择，
//! 未设置时回退 stub；第 5 步（曲线/报告端点、会话状态推导、重命名/删除/附件上传、
//! metrics 真实值）已落地。

mod adapters;
mod agent;
mod api;
mod application;
mod db;
mod tools;

use application::AppState;
#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("错误: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();

    let config = agent::LlmConfig::from_env();
    eprintln!("[config] provider = {}, model = {}", config.provider, config.model);

    // SQLite 连接池 + 嵌入式迁移。
    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://backend.db".to_string());
    let pool = db::establish_pool(&database_url)?;
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn)?;
    }
    eprintln!("[db] ready at {database_url}");

    // 全局状态：DB 仓储 / 事件总线 / 引擎适配器（env 选择）/ 限流信号量。
    let engine = adapters::engine_from_env();
    eprintln!(
        "[config] engine = {}",
        if std::env::var("TENSOREVAL_BIN").is_ok() {
            "process"
        } else {
            "stub"
        }
    );
    let state = AppState::new(pool, engine, config);

    let app = api::router().with_state(state);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[http] listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
