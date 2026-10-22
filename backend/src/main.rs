use backend::agent::LlmConfig;
use backend::application::AppState;
use backend::config::RuntimeConfig;
use backend::{adapters, build_router, db};

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("错误: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    let runtime = RuntimeConfig::from_env()?;
    let llm = LlmConfig::from_env();
    eprintln!(
        "[config] provider = {}, model = {}",
        llm.provider, llm.model
    );

    let pool = db::establish_pool(&runtime.database_url)?;
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn)?;
    }
    eprintln!("[db] ready at {}", runtime.database_url);

    let engine = adapters::engine_from_env();
    eprintln!(
        "[config] engine = {}",
        if std::env::var("TENSOREVAL_BIN").is_ok() {
            "process"
        } else {
            "stub"
        }
    );
    let state = AppState::new_with_data_dir(pool, engine, llm, runtime.data_dir);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[http] listening on http://{addr}");
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
