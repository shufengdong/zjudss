mod common;

use std::collections::BTreeMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::RunQueryDsl;
use tower::ServiceExt;
use zjudss::config::RuntimeConfig;

use common::TestApp;

#[test]
fn runtime_config_uses_v4_defaults_and_rejects_zero_or_inconsistent_limits() {
    let root = tempfile::tempdir().expect("create config root");
    let sparrowzz_dir = root.path().join("sparrowzz");
    std::fs::create_dir_all(sparrowzz_dir.join("dss/docs")).expect("create engine fixture");

    let config =
        RuntimeConfig::from_values(root.path(), BTreeMap::new(), Some(sparrowzz_dir.clone()))
            .expect("parse defaults");

    assert_eq!(config.database_url, "sqlite://backend.db");
    assert_eq!(config.bind_addr.to_string(), "127.0.0.1:8080");
    assert_eq!(config.data_dir, root.path().join("data"));
    assert_eq!(config.sparrowzz_dir, sparrowzz_dir);
    assert_eq!(
        config.session_ttl,
        std::time::Duration::from_secs(30 * 24 * 60 * 60)
    );
    assert!(!config.session_cookie_secure);
    assert_eq!(config.agent_max_concurrent_runs, 8);
    assert_eq!(config.agent_max_user_runs, 2);
    assert_eq!(config.agent_queue_capacity, 32);
    assert_eq!(config.upload_single_file_limit_bytes, 50 * 1024 * 1024);
    assert_eq!(config.upload_batch_limit_bytes, 500 * 1024 * 1024);
    assert_eq!(config.archive_max_entries, 1_000);
    assert_eq!(config.archive_max_expanded_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(config.archive_max_compression_ratio, 100);
    assert_eq!(config.user_artifact_limit, 20_000);
    assert_eq!(config.user_storage_limit_bytes, 20 * 1024 * 1024 * 1024);
    assert_eq!(config.storage_warning_ratio, 0.8);
    assert_eq!(config.upload_staging_ttl_hours, 24);
    assert_eq!(config.result_max_files, 100);
    assert_eq!(config.result_max_file_bytes, 512 * 1024 * 1024);
    assert_eq!(config.result_max_total_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(config.engine_process_output_max_bytes, 1024 * 1024);
    assert_eq!(config.script_repair_stderr_max_bytes, 64 * 1024);
    assert_eq!(config.script_repair_max_attempts, 3);
    assert_eq!(config.agent_model_retry_max, 2);
    assert_eq!(config.tensoreval_timeout_ms, 120_000);
    assert_eq!(config.agent_bundle_version, "v4.0.0");

    for name in [
        "RESULT_MAX_FILES",
        "SCRIPT_REPAIR_MAX_ATTEMPTS",
        "TENSOREVAL_TIMEOUT_MS",
    ] {
        let error = RuntimeConfig::from_values(
            root.path(),
            BTreeMap::from([(name.into(), "0".into())]),
            Some(sparrowzz_dir.clone()),
        )
        .expect_err("zero must fail");
        assert_eq!(error.code(), "config_invalid_value");
    }
    let error = RuntimeConfig::from_values(
        root.path(),
        BTreeMap::from([
            ("RESULT_MAX_FILE_BYTES".into(), "20".into()),
            ("RESULT_MAX_TOTAL_BYTES".into(), "10".into()),
        ]),
        Some(sparrowzz_dir.clone()),
    )
    .expect_err("total result budget must cover one file");
    assert_eq!(error.code(), "config_invalid_value");

    let error = RuntimeConfig::from_values(
        root.path(),
        BTreeMap::from([
            ("UPLOAD_SINGLE_FILE_LIMIT_BYTES".into(), "20".into()),
            ("UPLOAD_BATCH_LIMIT_BYTES".into(), "10".into()),
        ]),
        Some(sparrowzz_dir.clone()),
    )
    .expect_err("upload batch budget must cover one file");
    assert_eq!(error.code(), "config_invalid_value");

    for obsolete in [
        "OWNER_ARTIFACT_LIMIT",
        "OWNER_STORAGE_LIMIT_BYTES",
        "AGENT_MAX_OWNER_RUNS",
    ] {
        let error = RuntimeConfig::from_values(
            root.path(),
            BTreeMap::from([(obsolete.into(), "7".into())]),
            Some(sparrowzz_dir.clone()),
        )
        .expect_err("V3 names must be rejected");
        assert_eq!(error.code(), "config_obsolete_name");
    }
}

#[test]
fn bind_address_accepts_custom_ports_and_rejects_invalid_values() {
    let root = tempfile::tempdir().expect("create config root");
    for address in ["127.0.0.1:809", "[::1]:809"] {
        let values = BTreeMap::from([("BIND_ADDR".to_string(), address.to_string())]);
        let config = RuntimeConfig::from_values(root.path(), values, Some(root.path().into()))
            .expect("accept custom bind address");
        assert_eq!(config.bind_addr.to_string(), address);
    }
    for address in ["", "localhost:809", "127.0.0.1:65536", "127.0.0.1"] {
        let values = BTreeMap::from([("BIND_ADDR".to_string(), address.to_string())]);
        let error = RuntimeConfig::from_values(root.path(), values, Some(root.path().into()))
            .expect_err("reject invalid bind address");
        assert_eq!(error.code(), "config_invalid_value");
    }
}

#[test]
fn production_config_requires_readable_sparrowzz_dir() {
    let root = tempfile::tempdir().expect("create config root");

    let missing = RuntimeConfig::from_values(
        root.path(),
        BTreeMap::from([(
            "TENSOREVAL_CWD".to_string(),
            root.path().display().to_string(),
        )]),
        None,
    )
    .expect_err("production config must require SPARROWZZ_DIR and reject the old alias");
    assert_eq!(missing.code(), "config_sparrowzz_dir_missing");

    let vars = BTreeMap::from([(
        "SPARROWZZ_DIR".to_string(),
        root.path().join("does-not-exist").display().to_string(),
    )]);
    let unreadable = RuntimeConfig::from_values(root.path(), vars, None)
        .expect_err("production config must reject an unreadable Sparrowzz root");
    assert_eq!(unreadable.code(), "config_sparrowzz_dir_unreadable");

    let sparrowzz_dir = root.path().join("sparrowzz");
    std::fs::create_dir_all(&sparrowzz_dir).expect("create Sparrowzz root");
    let valid = RuntimeConfig::from_values(
        root.path(),
        BTreeMap::from([(
            "SPARROWZZ_DIR".to_string(),
            sparrowzz_dir.display().to_string(),
        )]),
        None,
    )
    .expect("production config accepts SPARROWZZ_DIR");
    assert_eq!(valid.sparrowzz_dir, sparrowzz_dir);
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
fn session_configuration_rejects_zero_ttl_and_invalid_secure_flag() {
    let root = tempfile::tempdir().expect("create config root");
    let engine_root = root.path().join("engine");
    std::fs::create_dir_all(&engine_root).expect("create engine root");
    let values = BTreeMap::from([
        ("SESSION_TTL_DAYS".to_string(), "0".to_string()),
        ("SESSION_COOKIE_SECURE".to_string(), "sometimes".to_string()),
    ]);

    let error = RuntimeConfig::from_values(root.path(), values, Some(engine_root))
        .expect_err("zero session TTL must be rejected");

    assert_eq!(error.code(), "config_invalid_value");
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
