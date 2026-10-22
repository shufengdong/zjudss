//! 配电网仿真智能体后端库入口。

pub mod adapters;
pub mod agent;
pub mod api;
pub mod application;
pub mod auth;
pub mod config;
pub mod db;
pub mod tools;

use axum::Router;

use application::AppState;

/// 构建已经绑定全局状态的 HTTP Router。
pub fn build_router(state: AppState) -> Router {
    api::router().with_state(state)
}
