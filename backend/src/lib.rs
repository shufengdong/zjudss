//! 配电网仿真智能体后端库入口。

pub mod adapters;
pub mod agent;
pub mod api;
pub mod application;
pub mod auth;
pub mod catalog;
pub mod config;
pub mod contract_manifest;
pub mod db;
pub mod engine_resources;
pub mod ingestion;
pub mod storage;
#[cfg(feature = "e2e-test")]
pub mod testing;
pub mod tools;

use axum::Router;

use application::AppState;

pub use contract_manifest::{ContractManifest, contract_manifest};

/// 构建已经绑定全局状态的 HTTP Router。
pub fn build_router(state: AppState) -> Router {
    api::router(state)
}
