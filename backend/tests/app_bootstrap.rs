mod common;

use std::collections::BTreeMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::config::{AuthMode, RuntimeConfig};
use diesel::RunQueryDsl;
use tower::ServiceExt;

use common::TestApp;

#[test]
fn config_defaults_are_safe() {
    let root = tempfile::tempdir().expect("create config root");
    let engine_root = root.path().join("engine");
    std::fs::create_dir_all(engine_root.join("sparrowzz/dss/docs")).expect("create engine fixture");

    let config =
        RuntimeConfig::from_values(root.path(), BTreeMap::new(), Some(engine_root.clone()))
            .expect("parse defaults");

    assert_eq!(config.database_url, "sqlite://backend.db");
    assert_eq!(config.data_dir, root.path().join("data"));
    assert_eq!(config.tensoreval_cwd, engine_root);
    assert_eq!(config.auth_mode, AuthMode::Instance);
    assert_eq!(config.agent_max_concurrent_runs, 8);
    assert_eq!(config.agent_max_owner_runs, 2);
    assert_eq!(config.agent_queue_capacity, 32);
    assert_eq!(config.upload_single_file_limit_bytes, 50 * 1024 * 1024);
    assert_eq!(config.upload_batch_limit_bytes, 500 * 1024 * 1024);
    assert_eq!(config.archive_max_entries, 1_000);
    assert_eq!(config.archive_max_expanded_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(config.archive_max_compression_ratio, 100);
    assert_eq!(config.owner_artifact_limit, 20_000);
    assert_eq!(config.owner_storage_limit_bytes, 20 * 1024 * 1024 * 1024);
    assert_eq!(config.storage_warning_ratio, 0.8);
    assert_eq!(config.agent_bundle_version, "v2.0.1");
}

#[test]
fn production_config_requires_readable_tensoreval_cwd() {
    let root = tempfile::tempdir().expect("create config root");

    let missing = RuntimeConfig::from_values(root.path(), BTreeMap::new(), None)
        .expect_err("production config must require TENSOREVAL_CWD");
    assert_eq!(missing.code(), "config_tensoreval_cwd_missing");

    let vars = BTreeMap::from([(
        "TENSOREVAL_CWD".to_string(),
        root.path().join("does-not-exist").display().to_string(),
    )]);
    let unreadable = RuntimeConfig::from_values(root.path(), vars, None)
        .expect_err("production config must reject an unreadable engine root");
    assert_eq!(unreadable.code(), "config_tensoreval_cwd_unreadable");
}

#[test]
fn config_rejects_zero_limits_and_unwritable_data_path() {
    let root = tempfile::tempdir().expect("create config root");
    let engine_root = root.path().join("engine");
    std::fs::create_dir_all(&engine_root).expect("create engine root");

    let zero_limit = RuntimeConfig::from_values(
        root.path(),
        BTreeMap::from([("AGENT_MAX_CONCURRENT_RUNS".to_string(), "0".to_string())]),
        Some(engine_root.clone()),
    )
    .expect_err("zero concurrency must be rejected");
    assert_eq!(zero_limit.code(), "config_invalid_value");

    let occupied = root.path().join("not-a-directory");
    std::fs::write(&occupied, b"x").expect("create occupied data path");
    let bad_data_dir = RuntimeConfig::from_values(
        root.path(),
        BTreeMap::from([("DATA_DIR".to_string(), occupied.display().to_string())]),
        Some(engine_root),
    )
    .expect_err("file cannot be used as DATA_DIR");
    assert_eq!(bad_data_dir.code(), "config_data_dir_unwritable");
}

#[test]
fn public_deployment_rejects_instance_auth() {
    let root = tempfile::tempdir().expect("create config root");
    let engine_root = root.path().join("engine");
    std::fs::create_dir_all(&engine_root).expect("create engine root");
    let values = BTreeMap::from([
        ("PUBLIC_DEPLOYMENT".to_string(), "true".to_string()),
        ("AUTH_MODE".to_string(), "instance".to_string()),
    ]);

    let error = RuntimeConfig::from_values(root.path(), values, Some(engine_root))
        .expect_err("public deployment must require trusted proxy auth");

    assert_eq!(error.code(), "config_public_auth_required");
}

#[tokio::test]
async fn test_app_runs_migrations_in_temp_dir() {
    let app = TestApp::spawn().await;

    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("build health request"),
        )
        .await
        .expect("health request");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024)
            .await
            .expect("health body"),
        "ok"
    );

    let mut conn = app.pool.get().expect("checkout migrated database");
    let rows = diesel::sql_query(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'sessions'",
    )
    .execute(&mut conn)
    .expect("query migrated schema");
    assert_eq!(rows, 1);
}
