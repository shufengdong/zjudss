use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::application::AppState;
use crate::auth::AuthConfig;
use crate::db;

use super::{EngineScenario, ScenarioEngine, ScriptScenario, ScriptedAgentFactory};

const CONTROL_HEADER: &str = "x-zjudss-e2e-token";

#[derive(Debug, Clone)]
pub struct E2eConfig {
    pub root: PathBuf,
    pub bind: SocketAddr,
    pub control_token: String,
    pub proxy_token: String,
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
        proxy_token: &str,
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
        if proxy_token.is_empty() {
            return Err(E2eConfigError::new(
                "e2e_proxy_token_missing",
                "E2E_PROXY_TOKEN 不能为空",
            ));
        }
        Ok(Self {
            root,
            bind,
            control_token: control_token.into(),
            proxy_token: proxy_token.into(),
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
            &std::env::var("E2E_PROXY_TOKEN").unwrap_or_default(),
        )
    }
}

#[derive(Clone)]
pub struct ControlState {
    token: Arc<str>,
    pub llm: Arc<ScriptedAgentFactory>,
    pub engine: Arc<ScenarioEngine>,
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
        }
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
        .route("/__e2e/engine-scenarios", post(enqueue_engine))
        .route("/__e2e/barriers/{id}/release", post(release))
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
async fn release(
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let released = state.llm.release(&id) | state.engine.release(&id);
    released
        .then(|| Json(json!({"released": true})))
        .ok_or(StatusCode::NOT_FOUND)
}
async fn trace(State(state): State<ControlState>) -> Json<Value> {
    Json(json!({"agent": state.llm.trace(), "engine": state.engine.calls()}))
}
async fn ready() -> Json<Value> {
    Json(json!({"ready": true}))
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
    }
    let llm = Arc::new(ScriptedAgentFactory::default());
    let engine = Arc::new(ScenarioEngine::default());
    let state = AppState::new_for_e2e(
        pool,
        data_dir,
        llm.clone(),
        engine.clone(),
        AuthConfig::trusted_proxy(config.proxy_token),
    );
    let router = crate::build_router(state).merge(control_router(ControlState::new(
        config.control_token,
        llm,
        engine,
    )));
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
