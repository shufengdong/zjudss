use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::AppState;
use crate::auth::{AuthError, CurrentUser};

use super::ApiError;

#[derive(Deserialize)]
pub struct LoginRequest {
    id: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let success = state
        .auth
        .login(request.id, request.password)
        .await
        .map_err(|error| match error {
            AuthError::InvalidCredentials | AuthError::AuthenticationRequired => {
                ApiError::invalid_credentials()
            }
            AuthError::Database => ApiError::internal("认证服务暂不可用"),
        })?;
    let cookie = HeaderValue::from_str(&success.set_cookie)
        .map_err(|_| ApiError::internal("认证 Cookie 无法编码"))?;
    let mut response = Json(json!({ "user": success.user })).into_response();
    response.headers_mut().append(header::SET_COOKIE, cookie);
    Ok(response)
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let cookie = state
        .auth
        .logout(&headers)
        .await
        .map_err(|_| ApiError::internal("认证服务暂不可用"))?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| ApiError::internal("认证 Cookie 无法编码"))?,
    );
    Ok(response)
}

pub async fn me(user: CurrentUser) -> Json<Value> {
    Json(json!({
        "id": user.id.0,
        "name": user.name,
    }))
}
