use backend::agent::knowledge::AgentKnowledge;
use backend::agent::{DeepSeekAgentFactory, LlmConfig};
use backend::application::{AppState, RunScheduler, StorageLimits};
use backend::auth::AuthConfig;
use backend::catalog::SchemaCatalog;
use backend::config::RuntimeConfig;
use backend::engine_resources::EngineResourceRegistry;
use backend::ingestion::ArchiveLimits;
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
    if std::env::args().nth(1).as_deref() == Some("rebuild-search-index") {
        let database_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://backend.db".into());
        let pool = db::establish_pool(&database_url)?;
        {
            let mut conn = pool.get()?;
            db::run_migrations(&mut conn)?;
        }
        backend::db::repositories::HistoryRepo::new(pool)
            .rebuild()
            .await?;
        eprintln!("[history] search index rebuilt");
        return Ok(());
    }
    let runtime = RuntimeConfig::from_env()?;
    let llm = LlmConfig::from_env();
    let knowledge = AgentKnowledge::load_embedded(&runtime.agent_bundle_version)?;
    let engine_resources = EngineResourceRegistry::load(&runtime.tensoreval_cwd)?;
    let snapshot_path = EngineResourceRegistry::publish(&engine_resources, &runtime.data_dir)?;
    let catalog = std::sync::Arc::new(SchemaCatalog::from_snapshot(
        &engine_resources,
        &engine_resources.contract_version,
    )?);
    let engine_resources = std::sync::Arc::new(engine_resources);
    eprintln!(
        "[config] provider = {}, model = {}",
        llm.provider, llm.model
    );
    eprintln!(
        "[config] agent bundle = {}, engine resources = {} ({})",
        knowledge.manifest.agent_bundle_version,
        engine_resources.id,
        snapshot_path.display()
    );

    let pool = db::establish_pool(&runtime.database_url)?;
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn)?;
        db::recover_interrupted_runs(&mut conn, backend::application::now_ms())?;
    }
    eprintln!("[db] ready at {}", runtime.database_url);

    let engine = adapters::engine_from_env()?;
    eprintln!("[config] engine = tensoreval parquet v2");
    let auth = AuthConfig::from_runtime(&runtime);
    let scheduler = RunScheduler::new(
        runtime.agent_max_concurrent_runs,
        runtime.agent_max_owner_runs,
        runtime.agent_queue_capacity,
    );
    let storage_limits = StorageLimits {
        artifact_limit: i64::try_from(runtime.owner_artifact_limit)?,
        byte_limit: i64::try_from(runtime.owner_storage_limit_bytes)?,
        warning_ratio: runtime.storage_warning_ratio,
    };
    let state = AppState::new_with_resources_and_agent_factory(
        pool,
        engine,
        std::sync::Arc::new(DeepSeekAgentFactory::new(llm)),
        runtime.data_dir,
        auth,
        scheduler,
        storage_limits,
        catalog,
        engine_resources,
        runtime.upload_single_file_limit_bytes,
        runtime.upload_batch_limit_bytes,
        ArchiveLimits {
            max_entries: runtime.archive_max_entries,
            max_expanded_bytes: runtime.archive_max_expanded_bytes,
            max_compression_ratio: runtime.archive_max_compression_ratio,
            max_file_bytes: runtime.upload_single_file_limit_bytes,
        },
    );

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[http] listening on http://{addr}");
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
