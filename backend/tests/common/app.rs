use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use tempfile::TempDir;
use zjudss::adapters::{SimEngine, StubEngine};
use zjudss::agent::knowledge::AgentKnowledge;
use zjudss::agent::{AgentFactory, FailureAnalyzer, LlmAgentFactory, LlmConfig, ScriptAuthor};
use zjudss::application::{AppState, RunScheduler};
use zjudss::auth::AuthService;
use zjudss::config::RuntimeConfig;
use zjudss::db::{self, DbPool};
use zjudss::testing::{
    E2eAppDependencies, E2eAppOptions, E2eAppProfile, E2eCatalogMode, assemble_e2e_app,
    prepare_engine_fixture,
};

use super::fixtures::seed_test_users;

const TEST_USERS: &[u8] = b"ID,NAME\ninstance,Instance\nalice,Alice\nbob,Bob\nforeign,Foreign\nalice@example.com,Alice Example\nbob@example.com,Bob Example\nowner:alice@example.com,Alice Owner\nowner:bob@example.com,Bob Owner\nowner:foreign,Foreign Owner\n";

pub struct TestApp {
    pub router: Router,
    pub pool: DbPool,
    pub state: AppState,
    pub live_engine_root: std::path::PathBuf,
    pub data_dir: std::path::PathBuf,
    _root: TempDir,
}

pub struct TestAppBuilder {
    ttl: Duration,
    secure: bool,
    scheduler: Option<RunScheduler>,
    engine: Arc<dyn SimEngine>,
    agent_factory: Arc<dyn AgentFactory>,
    failure_analyzer: Option<Arc<dyn FailureAnalyzer>>,
    script_author: Option<Arc<dyn ScriptAuthor>>,
    catalog_mode: E2eCatalogMode,
}

impl Default for TestAppBuilder {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(30 * 24 * 60 * 60),
            secure: false,
            scheduler: None,
            engine: Arc::new(StubEngine),
            agent_factory: Arc::new(LlmAgentFactory::new(LlmConfig {
                provider: "deepseek".to_string(),
                model: "test-model".to_string(),
            })),
            failure_analyzer: None,
            script_author: None,
            catalog_mode: E2eCatalogMode::Full,
        }
    }
}

impl TestAppBuilder {
    pub fn auth_settings(mut self, ttl: Duration, secure: bool) -> Self {
        self.ttl = ttl;
        self.secure = secure;
        self
    }

    pub fn scheduler(mut self, scheduler: RunScheduler) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub fn engine(mut self, engine: Arc<dyn SimEngine>) -> Self {
        self.engine = engine;
        self
    }

    pub fn agent_factory(mut self, agent_factory: Arc<dyn AgentFactory>) -> Self {
        self.agent_factory = agent_factory;
        self
    }

    pub fn failure_analyzer(mut self, failure_analyzer: Arc<dyn FailureAnalyzer>) -> Self {
        self.failure_analyzer = Some(failure_analyzer);
        self
    }

    pub fn script_author(mut self, script_author: Arc<dyn ScriptAuthor>) -> Self {
        self.script_author = Some(script_author);
        self
    }

    pub fn catalog_mode(mut self, catalog_mode: E2eCatalogMode) -> Self {
        self.catalog_mode = catalog_mode;
        self
    }

    pub async fn spawn(self) -> TestApp {
        let root = tempfile::tempdir().expect("create isolated test root");
        let data_dir = root.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("create isolated test data directory");
        let (engine_snapshot, live_engine_root) =
            prepare_engine_fixture(&data_dir, self.catalog_mode);
        let config = RuntimeConfig::from_values(
            root.path(),
            BTreeMap::from([(
                "DATABASE_URL".to_string(),
                root.path().join("test.db").display().to_string(),
            )]),
            Some(live_engine_root.clone()),
        )
        .expect("build isolated test config");
        AgentKnowledge::load_embedded(&config.agent_bundle_version)
            .expect("preflight embedded agent knowledge");

        let pool = db::establish_pool(&config.database_url).expect("create test database pool");
        {
            let mut connection = pool.get().expect("checkout test database");
            db::run_migrations(&mut connection).expect("run test migrations");
        }
        zjudss::db::repositories::EngineResourceSnapshotRepo::new(pool.clone())
            .publish_current(&engine_snapshot, zjudss::application::now_ms())
            .await
            .expect("publish test engine resource snapshot");
        seed_test_users(&pool, TEST_USERS);

        let minimal = self.catalog_mode == E2eCatalogMode::Minimal;
        let scheduler = self.scheduler.unwrap_or_else(|| {
            if minimal {
                RunScheduler::new(4, 2, 16)
            } else {
                RunScheduler::new(8, 2, 32)
            }
        });
        let profile = if minimal {
            E2eAppProfile::Minimal
        } else {
            E2eAppProfile::Runtime(Box::new(config.clone()))
        };
        let state = assemble_e2e_app(
            E2eAppDependencies {
                pool: pool.clone(),
                data_dir: config.data_dir.clone(),
                engine: self.engine,
                agent_factory: self.agent_factory,
                auth: AuthService::new(pool.clone(), self.ttl, self.secure),
                engine_snapshot,
            },
            E2eAppOptions {
                scheduler,
                profile,
                script_author: self.script_author,
                failure_analyzer: self.failure_analyzer,
                managed_workflow_auto_start: false,
                catalog_mode: self.catalog_mode,
            },
        );
        let router = zjudss::build_router(state.clone());

        TestApp {
            router,
            pool,
            state,
            live_engine_root,
            data_dir: config.data_dir,
            _root: root,
        }
    }
}

impl TestApp {
    pub fn builder() -> TestAppBuilder {
        TestAppBuilder::default()
    }

    pub async fn spawn() -> Self {
        Self::builder().spawn().await
    }

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
}
