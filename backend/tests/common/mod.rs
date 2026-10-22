use std::collections::BTreeMap;

use axum::Router;
use backend::adapters::StubEngine;
use backend::agent::LlmConfig;
use backend::application::{AppState, RunScheduler};
use backend::auth::AuthConfig;
use backend::config::RuntimeConfig;
use backend::db::{self, DbPool};
use tempfile::TempDir;

pub struct TestApp {
    pub router: Router,
    pub pool: DbPool,
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
        std::fs::create_dir_all(engine_root.join("sparrowzz/dss/docs"))
            .expect("create test engine resource fixture");

        let config = RuntimeConfig::from_values(
            root.path(),
            BTreeMap::from([(
                "DATABASE_URL".to_string(),
                root.path().join("test.db").display().to_string(),
            )]),
            Some(engine_root),
        )
        .expect("build isolated test config");
        let pool = db::establish_pool(&config.database_url).expect("create test database pool");
        {
            let mut conn = pool.get().expect("checkout test database");
            db::run_migrations(&mut conn).expect("run test migrations");
        }

        let state = AppState::new_with_data_dir_auth_and_scheduler(
            pool.clone(),
            std::sync::Arc::new(StubEngine),
            LlmConfig {
                provider: "deepseek".to_string(),
                model: "test-model".to_string(),
            },
            config.data_dir.clone(),
            auth,
            scheduler,
        );
        let router = backend::build_router(state);

        Self {
            router,
            pool,
            _root: root,
        }
    }
}
