//! 登录会话 HTTP 边界。
//!
//! 登录和登出是公开路由，以便创建或清除 HttpOnly 会话 Cookie；`/api/me` 由统一认证中间件
//! 保护。handler 只把 JSON/header 映射到 [`crate::auth`] 服务，不拥有密码校验、会话持久化
//! 或撤销广播规则。

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

/// `POST /api/auth/login`：校验固定用户凭据并创建登录会话。
///
/// 该公开接口接收用户 ID 和密码；成功返回 200、当前用户信息和 `Set-Cookie`。无效凭据统一
/// 返回 401 `invalid_credentials`，认证基础设施故障不暴露内部诊断。
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

/// `POST /api/auth/logout`：撤销请求 Cookie 对应的登录会话。
///
/// 该公开接口成功返回 204，并通过 `Set-Cookie` 清除浏览器 Cookie；会话撤销及 SSE 断开由
/// auth service 负责。
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

/// `GET /api/me`：返回当前已认证用户的公开身份。
///
/// 统一认证中间件先校验会话；成功返回 200 和 `{id,name}`，未登录返回 401
/// `authentication_required`，该读取不修改会话或用户数据。
pub async fn me(user: CurrentUser) -> Json<Value> {
    Json(json!({
        "id": user.id.0,
        "name": user.name,
    }))
}
