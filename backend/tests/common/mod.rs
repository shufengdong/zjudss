#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};

use axum::Router;
use backend::adapters::{SimEngine, StubEngine};
use backend::agent::knowledge::AgentKnowledge;
use backend::agent::{AgentFactory, DeepSeekAgentFactory, LlmConfig};
use backend::application::{AppState, RunScheduler};
use backend::auth::{AuthService, CurrentUser, UserId};
use backend::catalog::SchemaCatalog;
use backend::config::RuntimeConfig;
use backend::db::{self, DbPool};
use backend::engine_resources::{EngineResourceRegistry, ResourceContract, SourceFileSpec};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub struct TestApp {
    pub router: Router,
    pub pool: DbPool,
    pub state: AppState,
    _root: TempDir,
}

#[derive(diesel::QueryableByName)]
pub struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
}

pub fn seed_test_users(pool: &DbPool, csv_bytes: &[u8]) -> backend::auth::UserSeedSummary {
    let mut conn = pool.get().expect("checkout database for user seed");
    backend::auth::seed_users(
        &mut conn,
        csv_bytes,
        backend::application::now_ms(),
        backend::application::StorageLimits::default(),
    )
    .expect("seed test users")
}

pub fn test_user(id: &str) -> CurrentUser {
    let (name, storage_scope_id) = match id {
        "instance" => ("Instance", 1),
        "alice" => ("Alice", 2),
        "bob" => ("Bob", 3),
        "foreign" => ("Foreign", 4),
        "alice@example.com" => ("Alice Example", 5),
        "bob@example.com" => ("Bob Example", 6),
        "owner:alice@example.com" => ("Alice Owner", 7),
        "owner:bob@example.com" => ("Bob Owner", 8),
        "owner:foreign" => ("Foreign Owner", 9),
        other => panic!("unknown seeded test user: {other}"),
    };
    CurrentUser {
        id: UserId(id.to_string()),
        name: name.to_string(),
        storage_scope_id,
        auth_session_hash: "test".to_string(),
    }
}

impl TestApp {
    pub async fn login_cookie(&self, id: &str) -> String {
        self.state
            .auth
            .login(id.to_string(), id.to_string())
            .await
            .expect("login seeded test user")
            .set_cookie
            .split(';')
            .next()
            .expect("session cookie pair")
            .to_string()
    }

    pub async fn spawn() -> Self {
        Self::spawn_with_auth_settings(std::time::Duration::from_secs(30 * 24 * 60 * 60), false)
            .await
    }

    pub async fn spawn_with_auth_settings(ttl: std::time::Duration, secure: bool) -> Self {
        Self::spawn_with_scheduler(ttl, secure, RunScheduler::new(8, 2, 32)).await
    }

    pub async fn spawn_with_scheduler(
        ttl: std::time::Duration,
        secure: bool,
        scheduler: RunScheduler,
    ) -> Self {
        Self::spawn_with_scheduler_and_agent_factory(
            ttl,
            secure,
            scheduler,
            std::sync::Arc::new(StubEngine),
            std::sync::Arc::new(DeepSeekAgentFactory::new(LlmConfig {
                provider: "deepseek".to_string(),
                model: "test-model".to_string(),
            })),
        )
        .await
    }

    pub async fn spawn_with_agent_factory(factory: std::sync::Arc<dyn AgentFactory>) -> Self {
        Self::spawn_with_scheduler_and_agent_factory(
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
            false,
            RunScheduler::new(8, 2, 32),
            std::sync::Arc::new(StubEngine),
            factory,
        )
        .await
    }

    pub async fn spawn_with_engine(engine: std::sync::Arc<dyn SimEngine>) -> Self {
        Self::spawn_with_scheduler_and_agent_factory(
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
            false,
            RunScheduler::new(8, 2, 32),
            engine,
            std::sync::Arc::new(DeepSeekAgentFactory::new(LlmConfig {
                provider: "deepseek".to_string(),
                model: "test-model".to_string(),
            })),
        )
        .await
    }

    async fn spawn_with_scheduler_and_agent_factory(
        ttl: std::time::Duration,
        secure: bool,
        scheduler: RunScheduler,
        engine: std::sync::Arc<dyn SimEngine>,
        agent_factory: std::sync::Arc<dyn AgentFactory>,
    ) -> Self {
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
        std::fs::create_dir_all(dss_root.join("models/demo")).unwrap();
        std::fs::create_dir_all(engine_root.join("psa/tests/bmopf")).unwrap();
        std::fs::create_dir_all(engine_root.join("sparrowzz/rspower/lib")).unwrap();
        let demo_rss = b"a=import_dss(\"sparrowzz/dss/models/demo/master.dss\");\nb=import_bmopf(\"psa/tests/bmopf/demo.json\");\nc=read(\"sparrowzz/rspower/lib/helper.csv\");\n";
        let master = b"Redirect child.dss\n";
        std::fs::write(dss_root.join("demo.rss"), demo_rss).unwrap();
        std::fs::write(dss_root.join("models/demo/master.dss"), master).unwrap();
        std::fs::write(
            dss_root.join("models/demo/child.dss"),
            b"New Circuit.demo\n",
        )
        .unwrap();
        std::fs::write(engine_root.join("psa/tests/bmopf/demo.json"), b"{}\n").unwrap();
        std::fs::write(
            engine_root.join("sparrowzz/rspower/lib/helper.csv"),
            b"id\n1\n",
        )
        .unwrap();

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
                root_rss: vec![SourceFileSpec {
                    path: "demo.rss".into(),
                    sha256: hash(demo_rss),
                }],
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
        let engine_snapshot = std::sync::Arc::new(engine_snapshot);
        let pool = db::establish_pool(&config.database_url).expect("create test database pool");
        {
            let mut conn = pool.get().expect("checkout test database");
            db::run_migrations(&mut conn).expect("run test migrations");
        }
        seed_test_users(
            &pool,
            b"ID,NAME\ninstance,Instance\nalice,Alice\nbob,Bob\nforeign,Foreign\nalice@example.com,Alice Example\nbob@example.com,Bob Example\nowner:alice@example.com,Alice Owner\nowner:bob@example.com,Bob Owner\nowner:foreign,Foreign Owner\n",
        );
        let auth = AuthService::new(pool.clone(), ttl, secure);

        let state = AppState::new_with_resources_and_agent_factory(
            pool.clone(),
            engine,
            agent_factory,
            config.data_dir.clone(),
            auth,
            scheduler,
            backend::application::StorageLimits::default(),
            catalog,
            engine_snapshot,
            config.upload_single_file_limit_bytes,
            config.upload_batch_limit_bytes,
            backend::ingestion::ArchiveLimits {
                max_entries: config.archive_max_entries,
                max_expanded_bytes: config.archive_max_expanded_bytes,
                max_compression_ratio: config.archive_max_compression_ratio,
                max_file_bytes: config.upload_single_file_limit_bytes,
            },
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
