use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{AppState, ArtifactService, StorageLimits};
use crate::auth::{AuthService, seed_users};
use crate::db::{self, DbPool};

use super::{
    AuthorTurn, EngineScenario, FaultInjector, FaultPoint, ScenarioEngine, ScriptScenario,
    ScriptedAgentFactory, ScriptedAuthor,
};

const CONTROL_HEADER: &str = "x-zjudss-e2e-token";

#[derive(Debug, Clone)]
pub struct E2eConfig {
    pub root: PathBuf,
    pub bind: SocketAddr,
    pub control_token: String,
}

#[derive(Debug)]
pub struct E2eConfigError {
    code: &'static str,
    message: String,
}

impl E2eConfigError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for E2eConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for E2eConfigError {}

impl E2eConfig {
    pub fn from_values(
        root: PathBuf,
        bind: &str,
        control_token: &str,
    ) -> Result<Self, E2eConfigError> {
        let bind = bind.parse::<SocketAddr>().map_err(|error| {
            E2eConfigError::new("e2e_bind_invalid", format!("E2E_BIND 无效: {error}"))
        })?;
        if !bind.ip().is_loopback() {
            return Err(E2eConfigError::new(
                "e2e_bind_not_loopback",
                "e2e server 只允许监听 loopback",
            ));
        }
        if control_token.is_empty() {
            return Err(E2eConfigError::new(
                "e2e_control_token_missing",
                "E2E_CONTROL_TOKEN 不能为空",
            ));
        }
        Ok(Self {
            root,
            bind,
            control_token: control_token.into(),
        })
    }

    pub fn from_env() -> Result<Self, E2eConfigError> {
        let root = std::env::var("E2E_ROOT")
            .map(PathBuf::from)
            .map_err(|_| E2eConfigError::new("e2e_root_missing", "缺少 E2E_ROOT"))?;
        let bind = std::env::var("E2E_BIND").unwrap_or_else(|_| "127.0.0.1:0".into());
        Self::from_values(
            root,
            &bind,
            &std::env::var("E2E_CONTROL_TOKEN").unwrap_or_default(),
        )
    }
}

#[derive(Clone)]
pub struct ControlState {
    token: Arc<str>,
    pub llm: Arc<ScriptedAgentFactory>,
    pub engine: Arc<ScenarioEngine>,
    pub author: Arc<ScriptedAuthor>,
    faults: FaultInjector,
    artifacts: Option<ArtifactService>,
    pool: Option<DbPool>,
}

impl ControlState {
    pub fn new(
        token: impl Into<String>,
        llm: Arc<ScriptedAgentFactory>,
        engine: Arc<ScenarioEngine>,
    ) -> Self {
        Self {
            token: Arc::from(token.into()),
            llm,
            engine,
            author: Arc::new(ScriptedAuthor::default()),
            faults: FaultInjector,
            artifacts: None,
            pool: None,
        }
    }
    pub fn with_author(mut self, author: Arc<ScriptedAuthor>) -> Self {
        self.author = author;
        self
    }
    pub fn with_app(mut self, state: &AppState) -> Self {
        self.artifacts = Some(state.artifact_service.clone());
        self.pool = Some(state.pool.clone());
        self
    }
    fn authorize(&self, headers: &axum::http::HeaderMap) -> Result<(), StatusCode> {
        let supplied = headers
            .get(CONTROL_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        constant_time_eq(supplied.as_bytes(), self.token.as_bytes())
            .then_some(())
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}

pub fn control_router(state: ControlState) -> Router {
    Router::new()
        .route("/__e2e/llm-scenarios", post(enqueue_llm))
        .route("/__e2e/author-turns", post(enqueue_author))
        .route("/__e2e/engine-scenarios", post(enqueue_engine))
        .route("/__e2e/barriers/{id}/release", post(release))
        .route("/__e2e/faults", post(inject_fault))
        .route("/__e2e/storage-limits", post(set_storage_limits))
        .route("/__e2e/data-resources", post(seed_data_resource))
        .route("/__e2e/history-index/drop", post(drop_history_index))
        .route("/__e2e/history-index/rebuild", post(rebuild_history_index))
        .route("/__e2e/trace", get(trace))
        .route("/__e2e/ready", get(ready))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_control_token,
        ))
        .with_state(state)
}

async fn require_control_token(
    State(state): State<ControlState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if let Err(status) = state.authorize(request.headers()) {
        return status.into_response();
    }
    next.run(request).await
}

async fn enqueue_llm(
    State(state): State<ControlState>,
    Json(scenario): Json<ScriptScenario>,
) -> (StatusCode, Json<Value>) {
    state.llm.enqueue(scenario);
    (StatusCode::ACCEPTED, Json(json!({"ok": true})))
}
async fn enqueue_engine(
    State(state): State<ControlState>,
    Json(scenario): Json<EngineScenario>,
) -> (StatusCode, Json<Value>) {
    state.engine.enqueue(scenario);
    (StatusCode::ACCEPTED, Json(json!({"ok": true})))
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum AuthorTurnRequest {
    Script { content: String },
    Error { message: String },
    Wait { barrier_id: String },
}
async fn enqueue_author(
    State(state): State<ControlState>,
    Json(turn): Json<AuthorTurnRequest>,
) -> (StatusCode, Json<Value>) {
    state.author.enqueue(match turn {
        AuthorTurnRequest::Script { content } => AuthorTurn::Script(content),
        AuthorTurnRequest::Error { message } => AuthorTurn::Error(message),
        AuthorTurnRequest::Wait { barrier_id } => AuthorTurn::Wait(barrier_id),
    });
    (StatusCode::ACCEPTED, Json(json!({"ok": true})))
}
async fn release(
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let released = state.llm.release(&id) | state.engine.release(&id) | state.author.release(&id);
    released
        .then(|| Json(json!({"released": true})))
        .ok_or(StatusCode::NOT_FOUND)
}
async fn trace(State(state): State<ControlState>) -> Json<Value> {
    Json(json!({
        "agent": state.llm.trace(),
        "author": state.author.trace(),
        "authorRepairCount": state.author.repair_requests().len(),
        "engine": state.engine.calls()
    }))
}
async fn ready() -> Json<Value> {
    Json(json!({"ready": true}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FaultRequest {
    point: String,
}

async fn inject_fault(
    State(state): State<ControlState>,
    Json(request): Json<FaultRequest>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    match request.point.as_str() {
        "event_commit" => state.faults.fail_next(FaultPoint::EventCommit),
        _ => return Err(StatusCode::UNPROCESSABLE_ENTITY),
    }
    Ok((StatusCode::ACCEPTED, Json(json!({"ok": true}))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StorageLimitsRequest {
    kind: String,
    key: String,
    artifact_limit: i64,
    byte_limit: i64,
    warning_ratio: f64,
}

async fn set_storage_limits(
    State(state): State<ControlState>,
    Json(request): Json<StorageLimitsRequest>,
) -> Result<Json<Value>, StatusCode> {
    if request.kind != "user" {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let artifacts = state.artifacts.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let pool = state.pool.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let user_id = request.key;
    let scope_id = tokio::task::spawn_blocking(move || {
        use diesel::prelude::*;
        let mut connection = pool.get().map_err(|_| ())?;
        crate::db::schema::users::table
            .find(user_id)
            .select(crate::db::schema::users::storage_scope_id)
            .first::<i32>(&mut connection)
            .map_err(|_| ())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    artifacts
        .set_limits(
            scope_id,
            StorageLimits {
                artifact_limit: request.artifact_limit,
                byte_limit: request.byte_limit,
                warning_ratio: request.warning_ratio,
            },
        )
        .await
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeedDataResourceRequest {
    user_id: String,
    session_id: i32,
    upload_batch_id: i32,
    resource_id: String,
}

async fn seed_data_resource(
    State(state): State<ControlState>,
    Json(request): Json<SeedDataResourceRequest>,
) -> Result<(StatusCode, Json<Value>), StatusCode> {
    let pool = state.pool.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::task::spawn_blocking(move || {
        use diesel::prelude::*;
        let mut connection = pool.get().map_err(|_| ())?;
        let owned = crate::db::schema::sessions::table
            .filter(crate::db::schema::sessions::id.eq(request.session_id))
            .filter(crate::db::schema::sessions::user_id.eq(request.user_id))
            .select(crate::db::schema::sessions::id)
            .first::<i32>(&mut connection)
            .optional()
            .map_err(|_| ())?;
        if owned.is_none() {
            return Err(());
        }
        let artifact_id = crate::db::schema::upload_candidates::table
            .filter(crate::db::schema::upload_candidates::batch_id.eq(request.upload_batch_id))
            .select(crate::db::schema::upload_candidates::normalized_artifact_id)
            .first::<Option<i32>>(&mut connection)
            .map_err(|_| ())?
            .ok_or(())?;
        diesel::insert_into(crate::db::schema::data_resources::table)
            .values(crate::db::models::DataResourceRecord {
                id: request.resource_id,
                session_id: request.session_id,
                display_name: "E2E resource".into(),
                alias: "e2e_resource".into(),
                normalized_alias: "e2e_resource".into(),
                status: "live".into(),
                declared_schema_ref: None,
                source_kind: "upload".into(),
                normalized_artifact_id: Some(artifact_id),
                source_artifact_id: None,
                schema_json: "[]".into(),
                schema_hash: "0".repeat(64),
                row_count: 1,
                source_batch_id: Some(request.upload_batch_id),
                source_model_version_id: None,
                replaced_by_resource_id: None,
                lineage_json: "{}".into(),
                created_at_ms: crate::application::now_ms(),
                updated_at_ms: crate::application::now_ms(),
                deleted_at_ms: None,
                tombstone_json: None,
            })
            .execute(&mut connection)
            .map_err(|_| ())?;
        Ok::<_, ()>(())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    Ok((StatusCode::CREATED, Json(json!({"ok":true}))))
}

async fn drop_history_index(State(state): State<ControlState>) -> Result<Json<Value>, StatusCode> {
    let pool = state.pool.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::task::spawn_blocking(move || {
        use diesel::RunQueryDsl;
        let mut connection = pool.get().map_err(|_| ())?;
        diesel::sql_query("DROP TABLE IF EXISTS chat_search_fts")
            .execute(&mut connection)
            .map_err(|_| ())?;
        Ok::<_, ()>(())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({"ok": true})))
}

async fn rebuild_history_index(
    State(state): State<ControlState>,
) -> Result<Json<Value>, StatusCode> {
    let pool = state.pool.ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::task::spawn_blocking(move || {
        use diesel::connection::SimpleConnection;
        let mut connection = pool.get().map_err(|_| ())?;
        connection
            .batch_execute(
                "CREATE VIRTUAL TABLE IF NOT EXISTS chat_search_fts USING fts5(search_text,content='chat_search_documents',content_rowid='id',tokenize='trigram');
                 INSERT INTO chat_search_fts(chat_search_fts) VALUES('rebuild');",
            )
            .map_err(|_| ())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({"ok": true})))
}

pub async fn run_e2e_server(
    config: E2eConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(&config.root)?;
    let data_dir = config.root.join("data");
    std::fs::create_dir_all(&data_dir)?;
    let database_path = config.root.join("e2e.db");
    let pool = db::establish_pool(&database_path.display().to_string())?;
    {
        let mut connection = pool.get()?;
        db::run_migrations(&mut connection)?;
        db::recover_interrupted_runs(&mut connection, crate::application::now_ms())?;
        seed_users(
            &mut connection,
            include_bytes!("../../../integration-tests/fixtures/init_users.csv"),
            crate::application::now_ms(),
            StorageLimits::default(),
        )?;
    }
    let llm = Arc::new(ScriptedAgentFactory::default());
    let engine = Arc::new(ScenarioEngine::default());
    let author = Arc::new(ScriptedAuthor::default());
    let state = AppState::new_for_e2e(
        pool.clone(),
        data_dir,
        llm.clone(),
        engine.clone(),
        AuthService::new(
            pool,
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
            false,
        ),
        crate::application::E2eAppOptions {
            script_author: Some(author.clone()),
            managed_workflow_auto_start: true,
            full_engine_catalog: true,
        },
    );
    crate::db::repositories::EngineResourceSnapshotRepo::new(state.pool.clone())
        .publish_current(&state.engine_snapshot, crate::application::now_ms())
        .await?;
    let control = ControlState::new(config.control_token, llm, engine)
        .with_author(author)
        .with_app(&state);
    let router = crate::build_router(state).merge(control_router(control));
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let address = listener.local_addr()?;
    println!(
        "E2E_READY {}",
        json!({"address": address.to_string(), "root": config.root.display().to_string()})
    );
    axum::serve(listener, router).await?;
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}
