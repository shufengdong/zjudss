mod seed;
mod trusted_proxy;

pub use seed::{INIT_USERS_CSV, UserSeedError, UserSeedSummary, seed_users};

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;

use crate::api::ApiError;
use crate::application::AppState;
use crate::config::{AuthMode, RuntimeConfig};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccessScope {
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageScopeRef {
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub access: AccessScope,
    pub storage: StorageScopeRef,
    pub subject: String,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    mode: AuthMode,
    proxy_token: Option<String>,
}

impl AuthConfig {
    pub fn instance() -> Self {
        Self {
            mode: AuthMode::Instance,
            proxy_token: None,
        }
    }

    pub fn trusted_proxy(token: impl Into<String>) -> Self {
        Self {
            mode: AuthMode::TrustedProxy,
            proxy_token: Some(token.into()),
        }
    }

    pub fn from_runtime(config: &RuntimeConfig) -> Self {
        Self {
            mode: config.auth_mode,
            proxy_token: config.auth_proxy_token.clone(),
        }
    }

    fn authenticate(&self, parts: &Parts) -> Result<Principal, AuthError> {
        match self.mode {
            AuthMode::Instance => Ok(Principal::instance()),
            AuthMode::TrustedProxy => trusted_proxy::authenticate(
                &parts.headers,
                self.proxy_token.as_deref().ok_or(AuthError::Unauthorized)?,
            ),
        }
    }
}

impl Principal {
    fn instance() -> Self {
        Self {
            access: AccessScope {
                kind: "instance".to_string(),
                key: "instance".to_string(),
            },
            storage: StorageScopeRef {
                kind: "instance".to_string(),
                key: "instance".to_string(),
            },
            subject: "instance".to_string(),
        }
    }
}

#[derive(Debug)]
enum AuthError {
    Unauthorized,
}

#[derive(Debug, Clone)]
pub struct CurrentPrincipal(pub Principal);

impl<S> FromRequestParts<S> for CurrentPrincipal
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        state
            .auth
            .authenticate(parts)
            .map(CurrentPrincipal)
            .map_err(|_| ApiError::unauthorized("认证失败"))
    }
}
