#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};

use axum::Router;
use backend::adapters::StubEngine;
use backend::agent::LlmConfig;
use backend::agent::knowledge::AgentKnowledge;
use backend::application::{AppState, RunScheduler};
use backend::auth::AuthConfig;
use backend::catalog::SchemaCatalog;
use backend::config::RuntimeConfig;
use backend::db::{self, DbPool};
use backend::engine_resources::{EngineResourceRegistry, ResourceContract};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub struct TestApp {
    pub router: Router,
    pub pool: DbPool,
    pub state: AppState,
    _root: TempDir,
}

impl TestApp {
    pub async fn spawn() -> Self {
        Self::spawn_with_auth(AuthConfig::instance()).await
    }

    pub async fn spawn_with_auth(auth: AuthConfig) -> Self {
        Self::spawn_with_scheduler(auth, RunScheduler::new(8, 2, 32)).await
    }

    pub async fn spawn_with_scheduler(auth: AuthConfig, scheduler: RunScheduler) -> Self {
        let root = tempfile::tempdir().expect("create isolated test root");
        let engine_root = root.path().join("engine");
        let dss_root = engine_root.join("sparrowzz/dss");
        std::fs::create_dir_all(dss_root.join("docs/catalog"))
            .expect("create test engine resource fixture");
        let dataframes = br#"{"schema_version":2,"frames":{"bus":{"required":{"id":"str"}}}}"#;
        let functions = br#"{"schema_version":2,"frame_functions":[]}"#;
        let guide = b"# Test engine guide\nfixture content\n";
        std::fs::write(dss_root.join("docs/catalog/dataframes.json"), dataframes).unwrap();
        std::fs::write(dss_root.join("docs/catalog/functions.json"), functions).unwrap();
        std::fs::write(dss_root.join("docs/guide.md"), guide).unwrap();

        let config = RuntimeConfig::from_values(
            root.path(),
            BTreeMap::from([(
                "DATABASE_URL".to_string(),
                root.path().join("test.db").display().to_string(),
            )]),
            Some(engine_root),
        )
        .expect("build isolated test config");
        AgentKnowledge::load_embedded(&config.agent_bundle_version)
            .expect("preflight embedded agent knowledge");
        let hash = |bytes: &[u8]| format!("{:x}", Sha256::digest(bytes));
        let mut docs_hash = Sha256::new();
        docs_hash.update(b"docs/guide.md");
        docs_hash.update(b"\0");
        docs_hash.update(guide);
        docs_hash.update(b"\0");
        let engine_snapshot = EngineResourceRegistry::load_with_contract(
            &config.tensoreval_cwd,
            ResourceContract {
                contract_version: "test-contract".into(),
                catalog_schema_version: 2,
                dataframes_sha256: hash(dataframes),
                functions_sha256: hash(functions),
                docs_tree_sha256: format!("{:x}", docs_hash.finalize()),
                doc_paths: vec!["docs/guide.md".into()],
                root_rss: vec![],
                bundles: vec![],
            },
            &HashSet::new(),
        )
        .expect("preflight isolated engine resource fixture");
        EngineResourceRegistry::publish(&engine_snapshot, &config.data_dir)
            .expect("publish isolated engine snapshot");
        let catalog = std::sync::Arc::new(
            SchemaCatalog::from_snapshot(&engine_snapshot, "test-contract")
                .expect("load isolated schema catalog"),
        );
        let pool = db::establish_pool(&config.database_url).expect("create test database pool");
        {
            let mut conn = pool.get().expect("checkout test database");
            db::run_migrations(&mut conn).expect("run test migrations");
        }

        let state = AppState::new_with_resources(
            pool.clone(),
            std::sync::Arc::new(StubEngine),
            LlmConfig {
                provider: "deepseek".to_string(),
                model: "test-model".to_string(),
            },
            config.data_dir.clone(),
            auth,
            scheduler,
            backend::application::StorageLimits::default(),
            catalog,
            config.upload_single_file_limit_bytes,
            config.upload_batch_limit_bytes,
        );
        let router = backend::build_router(state.clone());

        Self {
            router,
            pool,
            state,
            _root: root,
        }
    }
}
