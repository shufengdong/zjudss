mod common;

use backend::application::{PublishDraftRequest, UploadBatch, UploadFile};
use backend::db::models::DraftRef;
use backend::storage::StageLimits;
use common::TestApp;
use std::io::Cursor;

async fn apply_csv(
    app: &TestApp,
    session_id: i32,
    client_id: &str,
    file_name: &str,
    bytes: &[u8],
    expected: Option<DraftRef>,
) -> DraftRef {
    let user = common::test_user("alice@example.com");
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(bytes), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            &user,
            session_id,
            client_id.into(),
            expected,
            UploadBatch {
                client_batch_id: client_id.into(),
                files: vec![UploadFile {
                    client_file_id: client_id.into(),
                    file_name: file_name.into(),
                    relative_path: None,
                    media_type: Some("text/csv".into()),
                    staged,
                }],
            },
        )
        .await
        .unwrap();
    let ready = app
        .state
        .model_import_service
        .process(&user, accepted.id.parse().unwrap())
        .await
        .unwrap();
    app.state
        .model_import_service
        .resolve_and_apply(
            &user,
            accepted.id.parse().unwrap(),
            expected,
            ready
                .candidates
                .into_iter()
                .map(|candidate| candidate.id)
                .collect(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_new_draft_copies_the_active_version_and_replaces_only_selected_tables() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "draft copy", 1)
        .await
        .unwrap();
    let first = apply_csv(&app, session.id, "bus-first", "bus.csv", b"id\n1\n", None).await;
    let with_line = apply_csv(
        &app,
        session.id,
        "line-first",
        "line.csv",
        b"id\nl1\n",
        Some(first),
    )
    .await;
    let published = app
        .state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: with_line.id,
                expected_draft_revision: with_line.revision,
                client_publish_id: "publish-two-tables".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(published.version.dataframes.len(), 2);

    let draft = apply_csv(&app, session.id, "bus-change", "bus.csv", b"id\n2\n", None).await;
    let workspace = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    let draft_snapshot = workspace.draft.unwrap();
    assert_eq!(
        draft_snapshot.draft.base_version_id,
        Some(published.version.version.id)
    );
    assert_eq!(draft_snapshot.draft.id, draft.id);
    assert_eq!(draft_snapshot.dataframes.len(), 2);
    assert_eq!(
        draft_snapshot
            .dataframes
            .iter()
            .map(|table| table.table_name.as_str())
            .collect::<Vec<_>>(),
        vec!["bus", "line"]
    );
}
