mod seed;
mod session;

pub use seed::{INIT_USERS_CSV, UserSeedError, UserSeedSummary, seed_users};
pub use session::{AuthError, AuthService, CurrentUser, LoginSuccess, PublicUser, UserId};

use axum::extract::{FromRequestParts, Request, State};
use axum::http::{HeaderValue, header, request::Parts};
use axum::middleware::Next;
use axum::response::Response;

use crate::api::ApiError;
use crate::application::AppState;

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

impl From<&CurrentUser> for Principal {
    fn from(user: &CurrentUser) -> Self {
        Self {
            access: AccessScope {
                kind: "user".to_string(),
                key: user.id.0.clone(),
            },
            storage: StorageScopeRef {
                kind: "user".to_string(),
                key: user.id.0.clone(),
            },
            subject: user.id.0.clone(),
        }
    }
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or_else(ApiError::authentication_required)
    }
}

#[derive(Debug, Clone)]
pub struct CurrentPrincipal(pub Principal);

impl<S> FromRequestParts<S> for CurrentPrincipal
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        Ok(Self(Principal::from(&user)))
    }
}

pub async fn require_user(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let authentication = state
        .auth
        .authenticate(request.headers())
        .await
        .map_err(|error| match error {
            AuthError::AuthenticationRequired | AuthError::InvalidCredentials => {
                ApiError::authentication_required()
            }
            AuthError::Database => ApiError::internal("认证服务暂不可用"),
        })?;
    request.extensions_mut().insert(authentication.user);
    let mut response = next.run(request).await;
    if let Some(cookie) = authentication.renewal_cookie {
        let value = HeaderValue::from_str(&cookie)
            .map_err(|_| ApiError::internal("认证 Cookie 无法编码"))?;
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    Ok(response)
}
