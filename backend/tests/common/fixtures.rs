use std::io::Cursor;

use zjudss::application::{CandidateDecision, CandidatePurpose, UploadFile, UploadSourceKind};
use zjudss::auth::{CurrentUser, UserId};
use zjudss::db::DbPool;
use zjudss::storage::StageLimits;

use super::TestApp;

pub fn seed_test_users(pool: &DbPool, csv_bytes: &[u8]) -> zjudss::auth::UserSeedSummary {
    let mut connection = pool.get().expect("checkout database for user seed");
    zjudss::auth::seed_users(
        &mut connection,
        csv_bytes,
        zjudss::application::now_ms(),
        zjudss::application::StorageLimits::default(),
    )
    .expect("seed test users")
}

pub fn test_user(id: &str) -> CurrentUser {
    let (name, storage_scope_id) = match id {
        "instance" => ("Instance", 1),
        "alice" => ("Alice", 2),
        "bob" => ("Bob", 3),
        "foreign" => ("Foreign", 4),
        "alice@example.com" => ("Alice Example", 5),
        "bob@example.com" => ("Bob Example", 6),
        "owner:alice@example.com" => ("Alice Owner", 7),
        "owner:bob@example.com" => ("Bob Owner", 8),
        "owner:foreign" => ("Foreign Owner", 9),
        other => panic!("unknown seeded test user: {other}"),
    };
    CurrentUser {
        id: UserId(id.to_string()),
        name: name.to_string(),
        storage_scope_id,
        auth_session_hash: "test".to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn publish_model_bytes(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    client_id: &str,
    file_name: &str,
    bytes: &[u8],
    expected: Option<zjudss::db::models::DraftRef>,
    only_table: Option<&str>,
) -> zjudss::db::models::DraftRef {
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(bytes), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            client_id.to_string(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: client_id.to_string(),
                file_name: file_name.to_string(),
                relative_path: Some(file_name.to_string()),
                media_type: None,
                staged,
            }],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let inferred_table = candidate
                .inferred_schema_ref
                .as_deref()
                .and_then(|value| value.strip_prefix("frames."))
                .or(candidate.proposed_alias.as_deref());
            let selected = only_table
                .map(|table| inferred_table == Some(table) || index == 0)
                .unwrap_or(candidate.purpose == Some(CandidatePurpose::ModelTable));
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if selected {
                    CandidatePurpose::ModelTable
                } else {
                    candidate.purpose.unwrap_or(CandidatePurpose::Skip)
                },
                selected,
                table_name: selected.then(|| {
                    only_table
                        .map(str::to_string)
                        .or_else(|| inferred_table.map(str::to_string))
                        .expect("selected model candidate has a table name")
                }),
                display_name: None,
                alias: None,
            }
        })
        .collect();
    let confirmation = app
        .state
        .data_resource_service
        .confirm_upload(
            user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            expected,
            client_id,
        )
        .await
        .unwrap();
    assert_eq!(confirmation.model_status, "published", "{confirmation:?}");
    let workspace = app
        .state
        .version_service
        .workspace(user, session_id)
        .await
        .unwrap();
    let draft = workspace.draft.expect("model publication creates a draft");
    zjudss::db::models::DraftRef {
        id: draft.draft.id,
        revision: draft.draft.revision,
    }
}
