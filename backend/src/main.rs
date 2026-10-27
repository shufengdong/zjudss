use zjudss::agent::knowledge::AgentKnowledge;
use zjudss::agent::{DeepSeekAgentFactory, LlmConfig};
use zjudss::application::{AppState, RunScheduler, StorageLimits};
use zjudss::auth::{AuthService, INIT_USERS_CSV, seed_users};
use zjudss::catalog::SchemaCatalog;
use zjudss::config::RuntimeConfig;
use zjudss::engine_resources::EngineResourceRegistry;
use zjudss::ingestion::ArchiveLimits;
use zjudss::{adapters, build_router, db};

#[tokio::main]
async fn main() {
    eprintln!("[version] zjudss backend git = {}", env!("ZJUDSS_GIT_HASH"));
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
        zjudss::db::repositories::HistoryRepo::new(pool)
            .rebuild()
            .await?;
        eprintln!("[history] search index rebuilt");
        return Ok(());
    }
    let runtime = RuntimeConfig::from_env()?;
    let llm = LlmConfig::from_env();
    let process_engine = adapters::ProcessEngine::from_env()?;
    let binary_sha256 = process_engine.binary_sha256().to_string();
    let resource_features = process_engine.capabilities().resource_features();
    let knowledge = AgentKnowledge::load_embedded(&runtime.agent_bundle_version)?;
    let engine_resources =
        EngineResourceRegistry::load(&runtime.sparrowzz_dir, &resource_features)?;
    let (engine_resources, snapshot_path) =
        EngineResourceRegistry::publish(engine_resources, &runtime.data_dir)?;
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
        "[config] agent bundle = {}, engine resources = {} ({}, examples discovered={}, importable={}, invalid={})",
        knowledge.manifest.agent_bundle_version,
        engine_resources.id,
        snapshot_path.display(),
        engine_resources.examples.len(),
        engine_resources
            .examples
            .iter()
            .filter(|example| example.importable)
            .count(),
        engine_resources
            .examples
            .iter()
            .filter(|example| !example.importable)
            .count(),
    );
    eprintln!("[config] tensoreval sha256 = {binary_sha256}");

    let storage_limits = StorageLimits {
        artifact_limit: i64::try_from(runtime.owner_artifact_limit)?,
        byte_limit: i64::try_from(runtime.owner_storage_limit_bytes)?,
        warning_ratio: runtime.storage_warning_ratio,
    };
    let pool = db::establish_pool(&runtime.database_url)?;
    {
        let mut conn = pool.get()?;
        db::run_migrations(&mut conn)?;
        db::recover_interrupted_runs(&mut conn, zjudss::application::now_ms())?;
        let seeded = seed_users(
            &mut conn,
            INIT_USERS_CSV,
            zjudss::application::now_ms(),
            storage_limits,
        )?;
        eprintln!(
            "[auth] users = {}, seed = {}",
            seeded.user_count,
            &seeded.seed_sha256[..12]
        );
    }
    eprintln!("[db] ready at {}", runtime.database_url);

    let engine: std::sync::Arc<dyn adapters::SimEngine> = std::sync::Arc::new(process_engine);
    eprintln!("[config] engine = tensoreval parquet v3");
    let auth = AuthService::new(
        pool.clone(),
        runtime.session_ttl,
        runtime.session_cookie_secure,
    );
    auth.cleanup_expired(zjudss::application::now_ms(), 10_000)
        .map_err(|_| std::io::Error::other("无法清理过期认证会话"))?;
    let scheduler = RunScheduler::new(
        runtime.agent_max_concurrent_runs,
        runtime.agent_max_owner_runs,
        runtime.agent_queue_capacity,
    );
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

    let addr = runtime.bind_addr;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[http] listening on http://{addr}");
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
