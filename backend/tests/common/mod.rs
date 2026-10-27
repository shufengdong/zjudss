#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};
use std::io::Cursor;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tower::ServiceExt;
use zjudss::adapters::{SimEngine, StubEngine};
use zjudss::agent::knowledge::AgentKnowledge;
use zjudss::agent::{AgentFactory, DeepSeekAgentFactory, LlmConfig};
use zjudss::application::{AppState, RunScheduler};
use zjudss::application::{CandidateDecision, CandidatePurpose, UploadFile, UploadSourceKind};
use zjudss::auth::{AuthService, CurrentUser, UserId};
use zjudss::catalog::SchemaCatalog;
use zjudss::config::RuntimeConfig;
use zjudss::db::{self, DbPool};
use zjudss::engine_resources::EngineResourceRegistry;
use zjudss::storage::StageLimits;

pub struct TestApp {
    pub router: Router,
    pub pool: DbPool,
    pub state: AppState,
    pub live_engine_root: std::path::PathBuf,
    _root: TempDir,
}

#[derive(diesel::QueryableByName)]
pub struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub count: i64,
}

pub async fn session_deletion_token(app: &TestApp, session_id: i32, cookie: &str) -> String {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session_id}/deletion-preview"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["impactToken"]
        .as_str()
        .unwrap()
        .to_string()
}

pub fn seed_test_users(pool: &DbPool, csv_bytes: &[u8]) -> zjudss::auth::UserSeedSummary {
    let mut conn = pool.get().expect("checkout database for user seed");
    zjudss::auth::seed_users(
        &mut conn,
        csv_bytes,
        zjudss::application::now_ms(),
        zjudss::application::StorageLimits::default(),
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

#[allow(clippy::too_many_arguments)]
pub async fn publish_model_bytes(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    client_id: &str,
    file_name: &str,
    bytes: &[u8],
    expected: Option<zjudss::db::models::DraftRef>,
    only_table: Option<&str>,
) -> zjudss::db::models::DraftRef {
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(bytes), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            client_id.to_string(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: client_id.to_string(),
                file_name: file_name.to_string(),
                relative_path: Some(file_name.to_string()),
                media_type: None,
                staged,
            }],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let inferred_table = candidate
                .inferred_schema_ref
                .as_deref()
                .and_then(|value| value.strip_prefix("frames."))
                .or(candidate.proposed_alias.as_deref());
            let selected = only_table
                .map(|table| inferred_table == Some(table) || index == 0)
                .unwrap_or(candidate.purpose == Some(CandidatePurpose::ModelTable));
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if selected {
                    CandidatePurpose::ModelTable
                } else {
                    candidate.purpose.unwrap_or(CandidatePurpose::Skip)
                },
                selected,
                table_name: selected.then(|| {
                    only_table
                        .map(str::to_string)
                        .or_else(|| inferred_table.map(str::to_string))
                        .expect("selected model candidate has a table name")
                }),
                display_name: None,
                alias: None,
            }
        })
        .collect();
    let confirmation = app
        .state
        .data_resource_service
        .confirm_upload(
            user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            expected,
            client_id,
        )
        .await
        .unwrap();
    assert_eq!(confirmation.model_status, "published", "{confirmation:?}");
    let workspace = app
        .state
        .version_service
        .workspace(user, session_id)
        .await
        .unwrap();
    let draft = workspace.draft.expect("model publication creates a draft");
    zjudss::db::models::DraftRef {
        id: draft.draft.id,
        revision: draft.draft.revision,
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
        let engine_root = root.path().join("sparrowzz");
        let dss_root = engine_root.join("dss");
        std::fs::create_dir_all(dss_root.join("docs/catalog"))
            .expect("create test engine resource fixture");
        let dataframes = include_bytes!("../fixtures/engine_resources/catalog/dataframes.json");
        let functions = include_bytes!("../fixtures/engine_resources/catalog/functions.json");
        let study_facades =
            include_bytes!("../fixtures/engine_resources/catalog/study_facades.json");
        let guide = b"# Test engine guide\nfixture content\n";
        std::fs::write(dss_root.join("docs/catalog/dataframes.json"), dataframes).unwrap();
        std::fs::write(dss_root.join("docs/catalog/functions.json"), functions).unwrap();
        std::fs::write(
            dss_root.join("docs/catalog/study_facades.json"),
            study_facades,
        )
        .unwrap();
        std::fs::write(dss_root.join("docs/guide.md"), guide).unwrap();
        std::fs::create_dir_all(dss_root.join("models/demo")).unwrap();
        std::fs::create_dir_all(engine_root.join("psa/tests/bmopf")).unwrap();
        std::fs::create_dir_all(engine_root.join("rspower/lib")).unwrap();
        let demo_rss = b"a=import_dss(\"dss/models/demo/master.dss\");\nb=import_bmopf(\"psa/tests/bmopf/demo.json\");\nc=read(\"rspower/lib/helper.csv\");\n";
        let master = b"Redirect child.dss\n";
        std::fs::write(dss_root.join("demo.rss"), demo_rss).unwrap();
        std::fs::write(dss_root.join("models/demo/master.dss"), master).unwrap();
        std::fs::write(
            dss_root.join("models/demo/child.dss"),
            b"New Circuit.demo\n",
        )
        .unwrap();
        std::fs::write(engine_root.join("psa/tests/bmopf/demo.json"), b"{}\n").unwrap();
        std::fs::write(engine_root.join("rspower/lib/helper.csv"), b"id\n1\n").unwrap();
        let bundle = dss_root.join("rssbundle/E2E/Demo/E2E demo.rssbundle");
        std::fs::create_dir_all(bundle.join("dss")).unwrap();
        let bundle_main = b"model = import_dss(\"dss/model.dss\");\n";
        let bundle_source = b"new circuit.e2e basekv=10\n";
        std::fs::write(bundle.join("main.rss"), bundle_main).unwrap();
        std::fs::write(bundle.join("dss/model.dss"), bundle_source).unwrap();
        let main_hash = hex::encode(Sha256::digest(bundle_main));
        let source_hash = hex::encode(Sha256::digest(bundle_source));
        let manifest = serde_json::json!({
            "schema_version": 1,
            "artifact": "mems.dss-rss-bundle",
            "generator": "zjudss-test/1",
            "psa_contract_version": "0.1.0",
            "entry": "main.rss",
            "study": {
                "kind": "execution_plan",
                "compatibility": "opendss-native",
                "solver": "radial",
                "results": ["voltage"]
            },
            "source": {
                "entry": "dss/model.dss",
                "root": "dss",
                "tree_sha256": source_hash
            },
            "files": [
                {"path": "dss/model.dss", "role": "dss-entry", "sha256": source_hash},
                {"path": "main.rss", "role": "main-rss", "sha256": main_hash}
            ],
            "inputs": []
        });
        std::fs::write(
            bundle.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let broken = dss_root.join("rssbundle/E2E/Demo/Broken.rssbundle");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("manifest.json"), b"{").unwrap();

        let live_engine_root = engine_root.clone();
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
        let engine_snapshot = EngineResourceRegistry::load(&config.sparrowzz_dir, &HashSet::new())
            .expect("preflight isolated engine resource fixture");
        let (engine_snapshot, _) =
            EngineResourceRegistry::publish(engine_snapshot, &config.data_dir)
                .expect("publish isolated engine snapshot");
        let catalog = std::sync::Arc::new(
            SchemaCatalog::from_snapshot(&engine_snapshot, &engine_snapshot.contract_version)
                .expect("load isolated schema catalog"),
        );
        let engine_snapshot = std::sync::Arc::new(engine_snapshot);
        let pool = db::establish_pool(&config.database_url).expect("create test database pool");
        {
            let mut conn = pool.get().expect("checkout test database");
            db::run_migrations(&mut conn).expect("run test migrations");
        }
        zjudss::db::repositories::EngineResourceSnapshotRepo::new(pool.clone())
            .publish_current(&engine_snapshot, zjudss::application::now_ms())
            .await
            .expect("publish test engine resource snapshot");
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
            zjudss::application::StorageLimits::default(),
            catalog,
            engine_snapshot,
            config.upload_single_file_limit_bytes,
            config.upload_batch_limit_bytes,
            config.upload_staging_ttl_hours,
            config.script_repair_max_attempts,
            config.agent_model_retry_max,
            zjudss::ingestion::ArchiveLimits {
                max_entries: config.archive_max_entries,
                max_expanded_bytes: config.archive_max_expanded_bytes,
                max_compression_ratio: config.archive_max_compression_ratio,
                max_file_bytes: config.upload_single_file_limit_bytes,
            },
        );
        let router = zjudss::build_router(state.clone());

        Self {
            router,
            pool,
            state,
            live_engine_root,
            _root: root,
        }
    }
}
