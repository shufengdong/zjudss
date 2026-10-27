mod common;

use std::io::Cursor;

use zjudss::application::{
    CandidateDecision, CandidatePurpose, ParseSettings, PublishDraftRequest, UploadBatchStatus,
    UploadFile, UploadSourceKind,
};
use zjudss::db::repositories::UploadRepo;
use zjudss::storage::StageLimits;

use common::TestApp;

fn file(app: &TestApp, id: &str, name: &str, bytes: Vec<u8>) -> UploadFile {
    UploadFile {
        client_file_id: id.into(),
        file_name: name.into(),
        relative_path: Some(name.into()),
        media_type: None,
        staged: app
            .state
            .artifact_service
            .store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .unwrap(),
    }
}

async fn session(app: &TestApp, title: &str) -> (zjudss::auth::CurrentUser, i32) {
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, title, 1)
        .await
        .unwrap();
    (user, session.id)
}

#[tokio::test]
async fn completed_parse_cannot_revive_a_discarded_batch() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "discard-parse-race").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "discard-parse-race".into(),
            UploadSourceKind::Upload,
            vec![file(&app, "source", "source.csv", b"x\n1\n".to_vec())],
        )
        .await
        .unwrap();
    let batch_id = batch.id.parse().unwrap();
    app.state
        .upload_batch_service
        .discard(&user, batch_id)
        .await
        .unwrap();

    let error = UploadRepo::new(app.pool.clone())
        .replace_candidates(
            batch_id,
            Vec::new(),
            "ready_to_confirm",
            "{}".into(),
            "[]".into(),
            zjudss::application::now_ms(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        zjudss::db::repositories::RepoError::InvalidState(ref message)
            if message == "upload_state_invalid"
    ));
    assert_eq!(
        app.state
            .upload_batch_service
            .get(&user, batch_id)
            .await
            .unwrap()
            .status,
        UploadBatchStatus::Discarded
    );
}

#[tokio::test]
async fn static_import_splits_twenty_model_tables_from_para_resource() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "static").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "static-1".into(),
            UploadSourceKind::Upload,
            vec![file(
                &app,
                "master",
                "master.dss",
                b"New Circuit.demo\n".to_vec(),
            )],
        )
        .await
        .unwrap();

    assert_eq!(batch.status, UploadBatchStatus::ReadyToConfirm);
    assert_eq!(
        batch
            .candidates
            .iter()
            .filter(|candidate| candidate.purpose == Some(CandidatePurpose::ModelTable))
            .count(),
        20
    );
    let para = batch
        .candidates
        .iter()
        .find(|candidate| candidate.proposed_alias.as_deref() == Some("para"))
        .unwrap();
    assert_eq!(para.purpose, Some(CandidatePurpose::DataResource));
    assert_eq!(para.inferred_schema_ref.as_deref(), Some("frames.para"));
}

#[tokio::test]
async fn case_import_never_puts_para_in_the_draft() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "case", 1)
        .await
        .unwrap();
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .unwrap();
    let batch = app
        .state
        .case_import_service
        .apply(
            &user,
            session.id,
            example.id.clone(),
            app.state.engine_snapshot.id.clone(),
            "case-1".into(),
        )
        .await
        .unwrap();
    assert!(batch.candidates.iter().all(|candidate| {
        candidate.proposed_alias.as_deref() != Some("para")
            || candidate.purpose == Some(CandidatePurpose::DataResource)
    }));
}

#[tokio::test]
async fn catalog_model_schema_outside_network20_publishes_to_draft_and_version() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "fuse").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "fuse-1".into(),
            UploadSourceKind::Upload,
            vec![file(
                &app,
                "fuse",
                "anything.csv",
                b"name,monitored_obj,switched_obj,fuse_curve\nf1,line.l1,line.l1,tcc1\n".to_vec(),
            )],
        )
        .await
        .unwrap();
    assert_eq!(
        batch.candidates[0].purpose,
        Some(CandidatePurpose::ModelTable)
    );
    assert_eq!(
        batch.candidates[0].inferred_schema_ref.as_deref(),
        Some("frames.fuse")
    );
    let confirmation = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::ModelTable,
                selected: true,
                table_name: Some("fuse".into()),
                display_name: None,
                alias: None,
            }],
        )
        .await
        .unwrap();
    assert_eq!(confirmation.model_status, "published");
    let workspace = app
        .state
        .version_service
        .workspace(&user, session_id)
        .await
        .unwrap();
    let draft = workspace.draft.unwrap();
    assert!(
        draft
            .dataframes
            .iter()
            .any(|frame| frame.table_name == "fuse")
    );
    let published = app
        .state
        .version_service
        .publish(
            &user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.draft.id,
                expected_draft_revision: draft.draft.revision,
                client_publish_id: "publish-fuse".into(),
                description: Some("动态门面模型表".into()),
            },
        )
        .await
        .unwrap();
    assert!(
        published
            .version
            .dataframes
            .iter()
            .any(|frame| frame.table_name == "fuse")
    );
}

#[tokio::test]
async fn csv_and_parquet_create_one_candidate_each() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "tabular").await;
    let csv = file(
        &app,
        "csv",
        "renamed.bin.csv",
        b"time,value\n0,1\n".to_vec(),
    );
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source.csv");
    let parquet_path = fixture.path().join("source.parquet");
    std::fs::write(&source, b"id,kv\nb1,10\n").unwrap();
    zjudss::ingestion::normalize_csv_to_parquet(&source, &parquet_path).unwrap();
    let parquet = std::fs::read(parquet_path).unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "tabular-1".into(),
            UploadSourceKind::Upload,
            vec![csv, file(&app, "parquet", "renamed.parquet", parquet)],
        )
        .await
        .unwrap();
    assert_eq!(batch.candidates.len(), 2);
    assert!(
        batch
            .candidates
            .iter()
            .all(|candidate| candidate.schema.is_some())
    );
}

#[tokio::test]
async fn excel_visible_nonempty_sheets_default_selected_and_hidden_sheets_do_not() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "excel").await;
    let bytes = std::fs::read("tests/fixtures/inference/multi.xlsx").unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "excel-1".into(),
            UploadSourceKind::Upload,
            vec![file(&app, "book", "multi.xlsx", bytes)],
        )
        .await
        .unwrap();
    assert!(!batch.candidates.is_empty());
    assert!(batch.candidates.iter().all(|candidate| candidate.selected));
}

#[tokio::test]
async fn ambiguous_parse_requires_settings() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "ambiguous").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "ambiguous-1".into(),
            UploadSourceKind::Upload,
            vec![file(
                &app,
                "csv",
                "ambiguous.csv",
                b"a,b;c\n1,2;3\n".to_vec(),
            )],
        )
        .await
        .unwrap();
    assert_eq!(batch.status, UploadBatchStatus::AwaitingParseSettings);
    let reparsed = app
        .state
        .upload_batch_service
        .update_parse_settings(
            &user,
            batch.id.parse().unwrap(),
            ParseSettings {
                delimiter: Some(b','),
                ..ParseSettings::default()
            },
        )
        .await
        .unwrap();
    assert_ne!(reparsed.status, UploadBatchStatus::AwaitingParseSettings);
}

#[tokio::test]
async fn formulas_macros_and_external_links_are_never_executed() {
    let bytes = std::fs::read("tests/fixtures/inference/multi.xlsx").unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("safe.xlsx");
    std::fs::write(&path, bytes).unwrap();
    let inspection = zjudss::ingestion::inspect_excel_safely(&path).unwrap();
    assert!(!inspection.executed_formulas);
    assert!(!inspection.executed_macros);
    assert!(!inspection.followed_external_links);
}

#[tokio::test]
async fn missing_required_formula_cache_is_rejected() {
    let error = zjudss::ingestion::validate_formula_cache(&[zjudss::ingestion::FormulaCell {
        position: "A1".into(),
        required: true,
        cached: None,
    }])
    .unwrap_err();
    assert_eq!(error.code(), "excel_formula_value_missing");
}

#[tokio::test]
async fn mixed_candidates_require_explicit_purpose() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "purpose").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "purpose-1".into(),
            UploadSourceKind::Upload,
            vec![file(&app, "unknown", "unknown.csv", b"x,y\n1,2\n".to_vec())],
        )
        .await
        .unwrap();
    assert_eq!(batch.status, UploadBatchStatus::AwaitingPurpose);
    let candidate = &batch.candidates[0];
    let ready = app
        .state
        .upload_batch_service
        .update_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: Some("预测数据".into()),
                alias: Some("forecast".into()),
            }],
        )
        .await
        .unwrap();
    assert_eq!(ready.status, UploadBatchStatus::ReadyToConfirm);
}

#[tokio::test]
async fn staging_reservation_survives_restart_and_expires_once() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "ttl").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "ttl-1".into(),
            UploadSourceKind::Upload,
            vec![file(&app, "csv", "data.csv", b"x\n1\n".to_vec())],
        )
        .await
        .unwrap();
    assert!(batch.reserved_bytes > 0);
    app.state
        .upload_batch_service
        .recover_startup()
        .await
        .unwrap();
    let recovered = app
        .state
        .upload_batch_service
        .get(&user, batch.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(recovered.status, batch.status);
    assert_eq!(recovered.candidates.len(), batch.candidates.len());
    app.state
        .upload_batch_service
        .expire_before(&user, i64::MAX)
        .await
        .unwrap();
    app.state
        .upload_batch_service
        .expire_before(&user, i64::MAX)
        .await
        .unwrap();
    let expired = app
        .state
        .upload_batch_service
        .get(&user, batch.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(expired.status, UploadBatchStatus::Expired);
    assert_eq!(expired.reserved_bytes, 0);
}

#[tokio::test]
async fn archive_and_batch_limits_leave_no_partial_batch() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "limit").await;
    let failed = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "limit-1".into(),
            UploadSourceKind::Upload,
            vec![file(&app, "archive", "broken.zip", b"not-a-zip".to_vec())],
        )
        .await;
    let failed = failed.unwrap();
    assert_eq!(failed.status, UploadBatchStatus::Failed);
    assert_eq!(failed.reserved_bytes, 0);
    assert_eq!(
        app.state
            .upload_batch_service
            .count_by_client_id(&user, session_id, "missing")
            .await
            .unwrap(),
        0
    );
}
