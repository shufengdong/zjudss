mod common;

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{Float64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use diesel::RunQueryDsl;
use parquet::arrow::ArrowWriter;

use zjudss::application::{
    ActivateVersionRequest, CandidateDecision, CandidatePurpose, PublishDraftRequest,
    ReplacementAction, UploadFile, UploadSourceKind,
};
use zjudss::db::models::DraftRef;
use zjudss::storage::StageLimits;

use common::TestApp;

fn csv(app: &TestApp, client_id: &str, name: &str, body: &[u8]) -> UploadFile {
    UploadFile {
        client_file_id: client_id.into(),
        file_name: name.into(),
        relative_path: Some(name.into()),
        media_type: Some("text/csv".into()),
        staged: app
            .state
            .artifact_service
            .store()
            .stage(Cursor::new(body), StageLimits::default())
            .unwrap(),
    }
}

fn numeric_parquet(app: &TestApp, client_id: &str, name: &str) -> UploadFile {
    let schema = Arc::new(Schema::new(vec![
        Field::new("time", DataType::Float64, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Float64Array::from(vec![0.0, 1.0])),
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
        ],
    )
    .unwrap();
    let mut bytes = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut bytes, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    UploadFile {
        client_file_id: client_id.into(),
        file_name: name.into(),
        relative_path: Some(name.into()),
        media_type: Some("application/vnd.apache.parquet".into()),
        staged: app
            .state
            .artifact_service
            .store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .unwrap(),
    }
}

async fn session(app: &TestApp, user: &zjudss::auth::CurrentUser, title: &str) -> i32 {
    app.state
        .session_repo
        .create(&user.id, title, 1)
        .await
        .unwrap()
        .id
}

async fn publish_csv_resource(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
    client_batch_id: &str,
    alias: &str,
    body: &[u8],
) -> zjudss::application::DataResourceView {
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            client_batch_id.into(),
            UploadSourceKind::Upload,
            vec![csv(app, client_batch_id, "resource.csv", body)],
        )
        .await
        .unwrap();
    app.state
        .data_resource_service
        .publish_candidates(
            user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some(alias.into()),
            }],
        )
        .await
        .unwrap()
        .resources
        .remove(0)
}

async fn publish_para_resource(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
    client_batch_id: &str,
) -> (zjudss::application::DataResourceView, i32) {
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            client_batch_id.into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "master".into(),
                file_name: "master.dss".into(),
                relative_path: Some("master.dss".into()),
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                    .unwrap(),
            }],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| {
            let is_para = candidate.inferred_schema_ref.as_deref() == Some("frames.para");
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if is_para {
                    CandidatePurpose::DataResource
                } else {
                    CandidatePurpose::Skip
                },
                selected: is_para,
                table_name: None,
                display_name: None,
                alias: is_para.then(|| "para".into()),
            }
        })
        .collect();
    let batch_id = batch.id.parse().unwrap();
    let published = app
        .state
        .data_resource_service
        .publish_candidates(user, batch_id, decisions)
        .await
        .unwrap();
    (published.resources[0].clone(), batch_id)
}

#[tokio::test]
async fn generic_dataframe_publishes_as_readable_resource_without_schema_ref() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "resources").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "resource-1".into(),
            UploadSourceKind::Upload,
            vec![csv(
                &app,
                "forecast",
                "forecast.csv",
                b"time,value\n0,1\n1,2\n",
            )],
        )
        .await
        .unwrap();
    let candidate = batch.candidates.first().unwrap();
    let decisions = vec![CandidateDecision {
        candidate_id: candidate.id.clone(),
        purpose: CandidatePurpose::DataResource,
        selected: true,
        table_name: None,
        display_name: Some("负荷预测".into()),
        alias: Some("load_forecast".into()),
    }];

    let confirmation = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            None,
            Some("confirm-resource-1"),
        )
        .await
        .unwrap();
    assert_eq!(confirmation.resources.len(), 1);
    assert!(confirmation.model_receipt.is_none());
    assert_eq!(
        confirmation.resource_receipt.as_ref().unwrap()["resourceIds"][0],
        confirmation.resources[0].id
    );
    let resource = &confirmation.resources[0];
    assert_eq!(resource.alias, "load_forecast");
    assert_eq!(resource.declared_schema_ref, None);
    assert!(resource.original_available);

    let page = app
        .state
        .data_resource_service
        .rows(&user, &resource.id, Some(10), None, vec![])
        .await
        .unwrap();
    assert_eq!(page.columns, ["time", "value"]);
    assert_eq!(page.rows.len(), 2);
    let replay = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            batch.id.parse().unwrap(),
            vec![],
            None,
            None,
            Some("confirm-resource-1"),
        )
        .await
        .unwrap();
    assert_eq!(replay.resources[0].id, resource.id);
    let conflict = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            batch.id.parse().unwrap(),
            vec![],
            None,
            None,
            Some("different-operation"),
        )
        .await
        .unwrap_err();
    assert_eq!(conflict.code(), "idempotency_conflict");
}

#[tokio::test]
async fn deleting_one_session_keeps_deduplicated_original_owned_by_another_session() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let first_session = session(&app, &user, "first-original").await;
    let second_session = session(&app, &user, "second-original").await;
    let body = b"time,value\n0,1\n";
    let first = publish_csv_resource(
        &app,
        &user,
        first_session,
        "same-source-first",
        "first_source",
        body,
    )
    .await;
    let second = publish_csv_resource(
        &app,
        &user,
        second_session,
        "same-source-second",
        "second_source",
        body,
    )
    .await;
    assert_eq!(first.content_hash, second.content_hash);

    let artifacts = app
        .state
        .session_repo
        .delete(&user.id, first_session)
        .await
        .unwrap();
    for artifact_id in artifacts {
        app.state
            .artifact_service
            .release(user.storage_scope_id, artifact_id)
            .await
            .unwrap();
    }
    assert!(
        app.state
            .data_resource_service
            .download(&user, &second.id, true)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn confirmed_batch_releases_skipped_candidate_artifacts() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "skip-cleanup").await;
    let (_para, batch_id) =
        publish_para_resource(&app, &user, session_id, "para-only-cleanup").await;

    let mut conn = app.pool.get().unwrap();
    let candidate_artifacts = diesel::sql_query(
        "SELECT count(*) count FROM upload_candidates WHERE batch_id=?1 AND normalized_artifact_id IS NOT NULL",
    )
    .bind::<diesel::sql_types::Integer, _>(batch_id)
    .get_result::<common::CountRow>(&mut conn)
    .unwrap();
    let file_artifacts = diesel::sql_query(
        "SELECT count(*) count FROM upload_files WHERE batch_id=?1 AND source_artifact_id IS NOT NULL",
    )
    .bind::<diesel::sql_types::Integer, _>(batch_id)
    .get_result::<common::CountRow>(&mut conn)
    .unwrap();
    assert_eq!(candidate_artifacts.count, 0);
    assert_eq!(file_artifacts.count, 0);
}

#[tokio::test]
async fn concurrent_confirmation_commits_each_original_reference_once() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "concurrent-confirm").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "concurrent-confirm".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "source", "source.csv", b"time,value\n0,1\n")],
        )
        .await
        .unwrap();
    let decisions = vec![CandidateDecision {
        candidate_id: batch.candidates[0].id.clone(),
        purpose: CandidatePurpose::DataResource,
        selected: true,
        table_name: None,
        display_name: None,
        alias: Some("concurrent_source".into()),
    }];
    let first = app.state.data_resource_service.confirm_upload(
        &user,
        batch.id.parse().unwrap(),
        decisions.clone(),
        None,
        None,
        Some("same-operation"),
    );
    let second = app.state.data_resource_service.confirm_upload(
        &user,
        batch.id.parse().unwrap(),
        decisions,
        None,
        None,
        Some("same-operation"),
    );
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.unwrap().resources.len(), 1);
    assert_eq!(second.unwrap().resources.len(), 1);

    #[derive(diesel::QueryableByName)]
    struct RefCount {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut conn = app.pool.get().unwrap();
    let excessive = diesel::sql_query(
        "SELECT count(*) count FROM storage_scope_artifacts WHERE storage_scope_id=?1 AND ref_count<>1",
    )
    .bind::<diesel::sql_types::Integer, _>(user.storage_scope_id)
    .get_result::<RefCount>(&mut conn)
    .unwrap();
    assert_eq!(excessive.count, 0);
}

#[tokio::test]
async fn confirm_and_discard_race_never_revives_a_terminal_batch() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "confirm-discard-race").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "confirm-discard-race".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "source", "source.csv", b"time,value\n0,1\n")],
        )
        .await
        .unwrap();
    let batch_id = batch.id.parse().unwrap();
    let decisions = vec![CandidateDecision {
        candidate_id: batch.candidates[0].id.clone(),
        purpose: CandidatePurpose::DataResource,
        selected: true,
        table_name: None,
        display_name: None,
        alias: Some("race_source".into()),
    }];

    let confirm = app.state.data_resource_service.confirm_upload(
        &user,
        batch_id,
        decisions,
        None,
        None,
        Some("confirm-discard-race"),
    );
    let discard = app.state.upload_batch_service.discard(&user, batch_id);
    let _ = tokio::join!(confirm, discard);

    let final_batch = app
        .state
        .upload_batch_service
        .get(&user, batch_id)
        .await
        .unwrap();
    assert!(matches!(
        final_batch.status,
        zjudss::application::UploadBatchStatus::Confirmed
            | zjudss::application::UploadBatchStatus::Discarded
    ));
    assert_eq!(final_batch.reserved_bytes, 0);

    let mut conn = app.pool.get().unwrap();
    let owned_candidate_artifacts = diesel::sql_query(
        "SELECT count(*) count FROM upload_candidates WHERE batch_id=?1 AND normalized_artifact_id IS NOT NULL",
    )
    .bind::<diesel::sql_types::Integer, _>(batch_id)
    .get_result::<common::CountRow>(&mut conn)
    .unwrap();
    assert_eq!(owned_candidate_artifacts.count, 0);
}

#[tokio::test]
async fn model_replacement_releases_old_draft_artifact_reference_atomically() {
    #[derive(diesel::QueryableByName)]
    struct ArtifactId {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        id: i32,
    }

    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "model-replacement").await;
    let first = common::publish_model_bytes(
        &app,
        &user,
        session_id,
        "model-first",
        "custom.csv",
        b"foo,bar\n1,old\n",
        None,
        Some("custom"),
    )
    .await;
    let old_artifact = {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query(
            "SELECT artifact_id id FROM draft_dataframes WHERE draft_id=?1 AND table_name='custom'",
        )
        .bind::<diesel::sql_types::Integer, _>(first.id)
        .get_result::<ArtifactId>(&mut conn)
        .unwrap()
        .id
    };

    let second = common::publish_model_bytes(
        &app,
        &user,
        session_id,
        "model-second",
        "custom.csv",
        b"foo,bar\n2,new\n",
        Some(first),
        Some("custom"),
    )
    .await;
    assert_eq!(second.revision, 2);

    let mut conn = app.pool.get().unwrap();
    let old_scope_refs = diesel::sql_query(
        "SELECT count(*) count FROM storage_scope_artifacts WHERE storage_scope_id=?1 AND artifact_id=?2",
    )
    .bind::<diesel::sql_types::Integer, _>(user.storage_scope_id)
    .bind::<diesel::sql_types::Integer, _>(old_artifact)
    .get_result::<common::CountRow>(&mut conn)
    .unwrap();
    assert_eq!(old_scope_refs.count, 0);
}

#[tokio::test]
async fn active_model_clone_counts_every_deduplicated_dataframe_reference() {
    #[derive(diesel::QueryableByName)]
    struct ArtifactRefCount {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        artifact_id: i32,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        ref_count: i64,
    }

    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "deduplicated-active-model").await;
    let initial = common::publish_model_bytes(
        &app,
        &user,
        session_id,
        "initial-model",
        "bus.csv",
        b"id,name\n1,source\n",
        None,
        Some("bus"),
    )
    .await;
    let published = app
        .state
        .version_service
        .publish(
            &user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: initial.id,
                expected_draft_revision: initial.revision,
                client_publish_id: "publish-shared-artifact".into(),
                description: Some("shared artifact fixture".into()),
            },
        )
        .await
        .unwrap();
    app.state
        .version_service
        .activate(
            &user,
            published.version.version.id,
            ActivateVersionRequest {
                discard_draft: false,
                expected_draft: None,
            },
        )
        .await
        .unwrap();

    let artifact_id = published.version.dataframes[0].artifact_id;
    {
        let frame = &published.version.dataframes[0];
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query("INSERT INTO model_dataframes(version_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) VALUES(?1,'shared_copy',?2,?3,?4,?5,?6)")
            .bind::<diesel::sql_types::Integer, _>(published.version.version.id)
            .bind::<diesel::sql_types::Integer, _>(artifact_id)
            .bind::<diesel::sql_types::Text, _>(&frame.schema_json)
            .bind::<diesel::sql_types::BigInt, _>(frame.row_count)
            .bind::<diesel::sql_types::Text, _>(&frame.content_hash)
            .bind::<diesel::sql_types::BigInt, _>(zjudss::application::now_ms())
            .execute(&mut conn)
            .unwrap();
        diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+1 WHERE storage_scope_id=?1 AND artifact_id=?2")
            .bind::<diesel::sql_types::Integer, _>(user.storage_scope_id)
            .bind::<diesel::sql_types::Integer, _>(artifact_id)
            .execute(&mut conn)
            .unwrap();
    }

    common::publish_model_bytes(
        &app,
        &user,
        session_id,
        "new-draft-model",
        "custom.csv",
        b"foo,bar\n1,new\n",
        None,
        Some("custom"),
    )
    .await;

    let mut conn = app.pool.get().unwrap();
    let counted = diesel::sql_query("SELECT artifact_id,ref_count FROM storage_scope_artifacts WHERE storage_scope_id=?1 AND artifact_id=?2")
        .bind::<diesel::sql_types::Integer, _>(user.storage_scope_id)
        .bind::<diesel::sql_types::Integer, _>(artifact_id)
        .get_result::<ArtifactRefCount>(&mut conn)
        .unwrap();
    assert_eq!(counted.artifact_id, artifact_id);
    assert_eq!(counted.ref_count, 4);
}

#[tokio::test]
async fn discarding_duplicate_candidates_releases_every_logical_reference() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "duplicate-discard").await;
    let baseline = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let body = b"time,value\n0,1\n";
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "duplicate-discard".into(),
            UploadSourceKind::Upload,
            vec![
                csv(&app, "first", "first.csv", body),
                csv(&app, "second", "second.csv", body),
            ],
        )
        .await
        .unwrap();
    app.state
        .upload_batch_service
        .discard(&user, batch.id.parse().unwrap())
        .await
        .unwrap();
    let after = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert_eq!(after.used_artifacts, baseline.used_artifacts);
    assert_eq!(after.used_bytes, baseline.used_bytes);
}

#[tokio::test]
async fn static_para_is_a_resource_and_never_a_model_dataframe() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "static").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "static-publish".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "master".into(),
                file_name: "master.dss".into(),
                relative_path: Some("master.dss".into()),
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                    .unwrap(),
            }],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: true,
            table_name: candidate
                .purpose
                .is_some_and(|p| p == CandidatePurpose::ModelTable)
                .then(|| candidate.proposed_alias.clone().unwrap()),
            display_name: None,
            alias: (candidate.proposed_alias.as_deref() == Some("para")).then(|| "para".into()),
        })
        .collect();
    let confirmation = app
        .state
        .data_resource_service
        .publish_candidates(&user, batch.id.parse().unwrap(), decisions)
        .await
        .unwrap();
    let para = confirmation
        .resources
        .iter()
        .find(|resource| resource.alias == "para")
        .unwrap();
    assert!(confirmation.model_receipt.is_some());
    assert!(confirmation.resource_receipt.is_some());
    assert_eq!(para.declared_schema_ref.as_deref(), Some("frames.para"));

    let workspace = app
        .state
        .version_service
        .workspace(&user, session_id)
        .await
        .unwrap();
    let draft = workspace.draft.unwrap();
    assert_eq!(draft.dataframes.len(), 20);
    assert!(
        draft
            .dataframes
            .iter()
            .all(|frame| frame.table_name != "para")
    );
}

#[tokio::test]
async fn repeated_schema_bound_para_requires_an_explicit_replacement_choice() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "para-replacement").await;

    for (client_batch_id, expected_status) in
        [("para-first", "published"), ("para-second", "failed")]
    {
        let batch = app
            .state
            .upload_batch_service
            .accept(
                &user,
                session_id,
                client_batch_id.into(),
                UploadSourceKind::Upload,
                vec![UploadFile {
                    client_file_id: "master".into(),
                    file_name: "master.dss".into(),
                    relative_path: Some("master.dss".into()),
                    media_type: None,
                    staged: app
                        .state
                        .artifact_service
                        .store()
                        .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                        .unwrap(),
                }],
            )
            .await
            .unwrap();
        let decisions = batch
            .candidates
            .iter()
            .map(|candidate| {
                let is_para = candidate.inferred_schema_ref.as_deref() == Some("frames.para");
                CandidateDecision {
                    candidate_id: candidate.id.clone(),
                    purpose: if is_para {
                        CandidatePurpose::DataResource
                    } else {
                        CandidatePurpose::Skip
                    },
                    selected: is_para,
                    table_name: None,
                    display_name: None,
                    alias: is_para.then(|| "para".into()),
                }
            })
            .collect();
        let confirmation = app
            .state
            .data_resource_service
            .publish_candidates(&user, batch.id.parse().unwrap(), decisions)
            .await
            .unwrap();
        assert_eq!(confirmation.resource_status, expected_status);
        if expected_status == "failed" {
            assert_eq!(
                confirmation.diagnostics[0]["code"],
                "resource_alias_conflict"
            );
        }
    }

    let resources = app
        .state
        .data_resource_service
        .list(&user, session_id, None, 20)
        .await
        .unwrap();
    assert_eq!(
        resources
            .resources
            .iter()
            .filter(|resource| resource.alias == "para")
            .count(),
        1
    );
}

#[tokio::test]
async fn schema_bound_replacement_can_be_saved_as_a_new_resource() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "para-save-as-new").await;
    let (old, _) = publish_para_resource(&app, &user, session_id, "para-original").await;
    let replacement = app
        .state
        .upload_batch_service
        .accept_replacement(
            &user,
            session_id,
            "para-copy".into(),
            old.id.clone(),
            vec![UploadFile {
                client_file_id: "master".into(),
                file_name: "master.dss".into(),
                relative_path: Some("master.dss".into()),
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                    .unwrap(),
            }],
        )
        .await
        .unwrap();
    let decisions = replacement
        .candidates
        .iter()
        .map(|candidate| {
            let is_para = candidate.inferred_schema_ref.as_deref() == Some("frames.para");
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if is_para {
                    CandidatePurpose::DataResource
                } else {
                    CandidatePurpose::Skip
                },
                selected: is_para,
                table_name: None,
                display_name: None,
                alias: None,
            }
        })
        .collect();
    let saved = app
        .state
        .data_resource_service
        .publish_replacement(
            &user,
            replacement.id.parse().unwrap(),
            decisions,
            ReplacementAction::SaveAsNew,
        )
        .await
        .unwrap();
    assert_eq!(saved.resource_status, "published");
    assert_eq!(saved.resources.len(), 1);
    assert!(saved.resources[0].alias.starts_with("data_"));
    assert_eq!(
        saved.resources[0].lineage["replacement"]["sourceResourceId"],
        old.id
    );
}

#[tokio::test]
async fn replacement_transfers_alias_and_preserves_old_artifact() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "replace").await;
    let first = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "first".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "first", "profile.csv", b"x\n1\n")],
        )
        .await
        .unwrap();
    let first_candidate = &first.candidates[0];
    let published = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            first.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: first_candidate.id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("profile".into()),
            }],
        )
        .await
        .unwrap();
    let old = published.resources[0].clone();
    {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query("INSERT OR IGNORE INTO engine_resource_snapshots(id,engine_contract_version,catalog_hash,functions_schema_version,study_facades_schema_version,dataframes_schema_version,dtype_grammar_version,is_current,snapshot_json,created_at_ms) VALUES(?1,'test',?2,1,1,1,1,1,'{}',1)")
            .bind::<diesel::sql_types::Text,_>(&app.state.engine_snapshot.id)
            .bind::<diesel::sql_types::Text,_>("0".repeat(64))
            .execute(&mut conn)
            .unwrap();
        diesel::sql_query("INSERT INTO study_configurations(session_id,facade_id,facade_hash,engine_snapshot_id,status,revision,intent_json,created_at_ms,updated_at_ms) VALUES(?1,'test.facade',?2,?3,'ready_to_confirm',1,'{}',1,1)")
            .bind::<diesel::sql_types::Integer,_>(session_id)
            .bind::<diesel::sql_types::Text,_>("0".repeat(64))
            .bind::<diesel::sql_types::Text,_>(&app.state.engine_snapshot.id)
            .execute(&mut conn)
            .unwrap();
        diesel::sql_query("INSERT INTO study_input_bindings(configuration_id,input_name,input_kind,resource_id,ordinal) VALUES((SELECT max(id) FROM study_configurations),'profile','data',?1,0)")
            .bind::<diesel::sql_types::Text,_>(&old.id)
            .execute(&mut conn)
            .unwrap();
    }
    let replacement = app
        .state
        .upload_batch_service
        .accept_replacement(
            &user,
            session_id,
            "second".into(),
            old.id.clone(),
            vec![csv(&app, "second", "profile.csv", b"x\n2\n")],
        )
        .await
        .unwrap();
    let candidate = &replacement.candidates[0];
    let replaced = app
        .state
        .data_resource_service
        .publish_replacement(
            &user,
            replacement.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: None,
            }],
            ReplacementAction::Replace,
        )
        .await
        .unwrap();
    assert_eq!(replaced.resources[0].alias, "profile");
    assert_ne!(replaced.resources[0].content_hash, old.content_hash);
    assert_eq!(
        replaced.resources[0].lineage["replacement"]["replacedResourceId"],
        old.id
    );
    {
        let mut conn = app.pool.get().unwrap();
        let invalidated = diesel::sql_query("SELECT count(*) count FROM study_configurations WHERE session_id=?1 AND status='invalidated'")
            .bind::<diesel::sql_types::Integer,_>(session_id)
            .get_result::<common::CountRow>(&mut conn)
            .unwrap();
        assert_eq!(invalidated.count, 1);
    }
    assert!(
        app.state
            .data_resource_service
            .historical(&user, &old.id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn foreign_resource_is_hidden_and_deletion_requires_current_impact_token() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let foreign = common::test_user("bob@example.com");
    let session_id = session(&app, &user, "delete").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "delete-1".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "one", "one.csv", b"x\n1\n")],
        )
        .await
        .unwrap();
    let candidate = &batch.candidates[0];
    let result = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("one".into()),
            }],
        )
        .await
        .unwrap();
    let id = &result.resources[0].id;
    let usage_before_delete = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert!(
        app.state
            .data_resource_service
            .get(&foreign, id)
            .await
            .is_err()
    );
    let preview = app
        .state
        .data_resource_service
        .preview_deletion(&user, id)
        .await
        .unwrap();
    app.state
        .data_resource_service
        .delete(&user, id, &preview.impact_token)
        .await
        .unwrap();
    let usage_after_delete = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert!(usage_after_delete.used_artifacts < usage_before_delete.used_artifacts);
    assert!(usage_after_delete.used_bytes < usage_before_delete.used_bytes);
    app.state
        .data_resource_service
        .delete(&user, id, &preview.impact_token)
        .await
        .unwrap();
    assert!(
        app.state
            .data_resource_service
            .get(&user, id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn structured_replacement_preview_does_not_publish_until_confirmed() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "structured").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "structured-source".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "master".into(),
                file_name: "master.dss".into(),
                relative_path: Some("master.dss".into()),
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                    .unwrap(),
            }],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: candidate.proposed_alias.as_deref() == Some("para"),
            table_name: None,
            display_name: None,
            alias: (candidate.proposed_alias.as_deref() == Some("para")).then(|| "para".into()),
        })
        .collect();
    let published = app
        .state
        .data_resource_service
        .publish_candidates(&user, batch.id.parse().unwrap(), decisions)
        .await
        .unwrap();
    let para = published.resources.first().unwrap();
    let before = app
        .state
        .data_resource_service
        .list(&user, session_id, None, 100)
        .await
        .unwrap()
        .resources
        .len();
    let preview=app.state.data_resource_service.preview_structured_replacement(&user,&para.id,&para.content_hash,"frames.para",serde_json::json!({"base_frequency_hz":50.0,"max_iter":30,"max_control_iter":10,"tol":1e-6,"line_model":"exact","result":"bus_phase,summary"})).await.unwrap();
    let first_preview_usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let superseding=app.state.data_resource_service.preview_structured_replacement(&user,&para.id,&para.content_hash,"frames.para",serde_json::json!({"base_frequency_hz":50.0,"max_iter":31,"max_control_iter":10,"tol":1e-6,"line_model":"exact","result":"bus_phase,summary"})).await.unwrap();
    let superseding_usage = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert_eq!(
        superseding_usage.used_artifacts,
        first_preview_usage.used_artifacts
    );
    let superseded = app
        .state
        .data_resource_service
        .confirm_structured_replacement(&user, &preview.preview_token, ReplacementAction::Replace)
        .await
        .unwrap_err();
    assert_eq!(superseded.code(), "replacement_preview_invalid");
    assert_eq!(
        app.state
            .data_resource_service
            .list(&user, session_id, None, 100)
            .await
            .unwrap()
            .resources
            .len(),
        before
    );
    let replacement = app
        .state
        .data_resource_service
        .confirm_structured_replacement(
            &user,
            &superseding.preview_token,
            ReplacementAction::Replace,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replacement.alias, "para");
    assert_ne!(replacement.content_hash, para.content_hash);
    let cancelled = app.state.data_resource_service.preview_structured_replacement(&user,&replacement.id,&replacement.content_hash,"frames.para",serde_json::json!({"base_frequency_hz":50.0,"max_iter":31,"max_control_iter":10,"tol":1e-6,"line_model":"exact","result":"bus_phase,summary"})).await.unwrap();
    assert!(
        app.state
            .data_resource_service
            .confirm_structured_replacement(
                &user,
                &cancelled.preview_token,
                ReplacementAction::Cancel,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        app.state
            .data_resource_service
            .list(&user, session_id, None, 100)
            .await
            .unwrap()
            .resources
            .len(),
        before
    );
    let save_as_new = app.state.data_resource_service.preview_structured_replacement(&user,&replacement.id,&replacement.content_hash,"frames.para",serde_json::json!({"base_frequency_hz":60.0,"max_iter":30,"max_control_iter":10,"tol":1e-6,"line_model":"exact","result":"bus_phase,summary"})).await.unwrap();
    let saved = app
        .state
        .data_resource_service
        .confirm_structured_replacement(
            &user,
            &save_as_new.preview_token,
            ReplacementAction::SaveAsNew,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(saved.alias.starts_with("data_"));
    assert_eq!(
        app.state
            .data_resource_service
            .list(&user, session_id, None, 100)
            .await
            .unwrap()
            .resources
            .len(),
        before + 1
    );
}

#[tokio::test]
async fn expired_and_stale_structured_previews_release_pending_artifacts() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "structured-preview-cleanup").await;
    let (para, _) = publish_para_resource(&app, &user, session_id, "preview-para").await;
    let values = serde_json::json!({
        "base_frequency_hz": 50.0,
        "max_iter": 31,
        "max_control_iter": 10,
        "tol": 1e-6,
        "line_model": "exact",
        "result": "bus_phase,summary"
    });
    let baseline = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let expired = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            &user,
            &para.id,
            &para.content_hash,
            "frames.para",
            values.clone(),
        )
        .await
        .unwrap();
    {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query("UPDATE data_resource_replacement_previews SET expires_at_ms=0")
            .execute(&mut conn)
            .unwrap();
    }
    assert_eq!(
        app.state
            .data_resource_service
            .cleanup_expired_structured_previews()
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        app.state
            .artifact_service
            .get_usage(user.storage_scope_id)
            .await
            .unwrap()
            .used_bytes,
        baseline.used_bytes
    );
    assert_eq!(
        app.state
            .data_resource_service
            .confirm_structured_replacement(
                &user,
                &expired.preview_token,
                ReplacementAction::Replace,
            )
            .await
            .unwrap_err()
            .code(),
        "replacement_preview_invalid"
    );

    let stale = app
        .state
        .data_resource_service
        .preview_structured_replacement(&user, &para.id, &para.content_hash, "frames.para", values)
        .await
        .unwrap();
    {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query("UPDATE data_resources SET status='replaced' WHERE id=?1")
            .bind::<diesel::sql_types::Text, _>(&para.id)
            .execute(&mut conn)
            .unwrap();
    }
    assert_eq!(
        app.state
            .data_resource_service
            .confirm_structured_replacement(
                &user,
                &stale.preview_token,
                ReplacementAction::Replace,
            )
            .await
            .unwrap_err()
            .code(),
        "resource_stale"
    );
    let mut conn = app.pool.get().unwrap();
    let pending = diesel::sql_query(
        "SELECT count(*) count FROM data_resource_replacement_previews WHERE old_resource_id=?1",
    )
    .bind::<diesel::sql_types::Text, _>(&para.id)
    .get_result::<common::CountRow>(&mut conn)
    .unwrap();
    assert_eq!(pending.count, 0);
}

#[tokio::test]
async fn partial_confirmation_keeps_only_failed_child_retryable() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "partial").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "partial-source".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "master".into(),
                file_name: "master.dss".into(),
                relative_path: Some("master.dss".into()),
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                    .unwrap(),
            }],
        )
        .await
        .unwrap();
    let mut decisions = batch
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: true,
            table_name: candidate.proposed_alias.clone(),
            display_name: None,
            alias: candidate.proposed_alias.clone(),
        })
        .collect::<Vec<_>>();
    let model = decisions
        .iter_mut()
        .find(|decision| decision.purpose == CandidatePurpose::ModelTable)
        .unwrap();
    model.table_name = Some("para".into());
    model.alias = None;
    let first = app
        .state
        .data_resource_service
        .publish_candidates(&user, batch.id.parse().unwrap(), decisions)
        .await
        .unwrap();
    assert_eq!(first.status, "partially_confirmed");
    assert_eq!(first.resource_status, "published");
    assert_eq!(first.model_status, "failed");
    assert_eq!(first.resources.len(), 1);
    let record = app
        .state
        .upload_batch_service
        .get(&user, batch.id.parse().unwrap())
        .await
        .unwrap();
    let retry = record
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: true,
            table_name: (candidate.purpose == Some(CandidatePurpose::ModelTable)).then(|| {
                candidate
                    .inferred_schema_ref
                    .as_deref()
                    .unwrap()
                    .trim_start_matches("frames.")
                    .to_string()
            }),
            display_name: None,
            alias: (candidate.purpose == Some(CandidatePurpose::DataResource))
                .then(|| candidate.proposed_alias.clone().unwrap()),
        })
        .collect();
    let second = app
        .state
        .data_resource_service
        .publish_candidates(&user, batch.id.parse().unwrap(), retry)
        .await
        .unwrap();
    assert_eq!(second.status, "confirmed");
    assert_eq!(second.resource_status, "published");
    assert_eq!(second.model_status, "published");
    assert_eq!(
        app.state
            .data_resource_service
            .list(&user, session_id, None, 100)
            .await
            .unwrap()
            .resources
            .len(),
        1
    );
}

#[tokio::test]
async fn ordinary_resource_can_feed_existing_analysis_pipeline_with_frozen_lineage() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "analysis-resource").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "analysis-input".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "source", "source.csv", b"x,value\n1,2\n")],
        )
        .await
        .unwrap();
    let candidate = &batch.candidates[0];
    let published = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("source".into()),
            }],
        )
        .await
        .unwrap();
    let resource = &published.resources[0];
    let analysis = app
        .state
        .analysis_service
        .run(zjudss::application::AnalysisRequest {
            principal: user.clone(),
            session_id,
            request_message_id: None,
            intent: serde_json::json!({"goal":"copy"}),
            script: "result = source".into(),
            inputs: vec![zjudss::application::AnalysisInput {
                source: zjudss::application::AnalysisInputSource::DataResourceId(
                    resource.id.clone(),
                ),
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            initial_views: Default::default(),
            skill_refs: vec![],
        })
        .await
        .unwrap();
    let metadata = app
        .state
        .dataset_service
        .metadata(&user, analysis.datasets[0].id)
        .await
        .unwrap();
    assert_eq!(
        metadata.lineage[0].source_resource_id.as_deref(),
        Some(resource.id.as_str())
    );
    assert_eq!(metadata.lineage[0].content_hash, resource.content_hash);
    let promoted = app
        .state
        .data_resource_service
        .promote_dataset(
            &user,
            session_id,
            analysis.datasets[0].id,
            "分析结果输入".into(),
        )
        .await
        .unwrap();
    assert_eq!(promoted.source_kind, "analysis");
    assert!(!promoted.original_available);
    assert_eq!(promoted.lineage["datasetId"], analysis.datasets[0].id);
    let deletion = app
        .state
        .data_resource_service
        .preview_deletion(&user, &resource.id)
        .await
        .unwrap();
    assert!(deletion.historical_reference_count >= 2);
    app.state
        .data_resource_service
        .delete(&user, &resource.id, &deletion.impact_token)
        .await
        .unwrap();
    let after_delete = app
        .state
        .dataset_service
        .metadata(&user, analysis.datasets[0].id)
        .await
        .unwrap();
    assert_eq!(
        after_delete.lineage[0].source_resource_id.as_deref(),
        Some(resource.id.as_str())
    );
    assert_eq!(after_delete.lineage[0].content_hash, resource.content_hash);
}

#[tokio::test]
async fn ordinary_resource_can_be_materialized_as_a_validated_view() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "resource-view").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "view-input".into(),
            UploadSourceKind::Upload,
            vec![numeric_parquet(&app, "view-source", "view-source.parquet")],
        )
        .await
        .unwrap();
    let candidate = &batch.candidates[0];
    let decisions = vec![CandidateDecision {
        candidate_id: candidate.id.clone(),
        purpose: CandidatePurpose::DataResource,
        selected: true,
        table_name: None,
        display_name: None,
        alias: Some("view_source".into()),
    }];
    let published = app
        .state
        .data_resource_service
        .publish_candidates(&user, batch.id.parse().unwrap(), decisions)
        .await
        .unwrap();
    let resource = &published.resources[0];
    let view = app
        .state
        .analysis_service
        .create_resource_view(
            &user,
            &resource.id,
            "负荷曲线".into(),
            serde_json::json!({"kind":"line","x":"time","y":["value"]}),
        )
        .await
        .unwrap();
    let metadata = app
        .state
        .dataset_service
        .metadata(&user, view.source_dataset_id)
        .await
        .unwrap();
    assert_eq!(
        metadata.lineage[0].source_resource_id.as_deref(),
        Some(resource.id.as_str())
    );
    assert_eq!(metadata.lineage[0].content_hash, resource.content_hash);
}

#[tokio::test]
async fn engine_snapshot_revalidation_marks_reserved_alias_until_user_renames_it() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "alias-revalidation").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "alias-revalidation".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "source", "source.csv", b"x\n1\n")],
        )
        .await
        .unwrap();
    let candidate = &batch.candidates[0];
    let published = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("safe_input".into()),
            }],
        )
        .await
        .unwrap();
    let resource_id = &published.resources[0].id;
    {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query(
            "UPDATE data_resources SET alias='result',normalized_alias='result' WHERE id=?1",
        )
        .bind::<diesel::sql_types::Text, _>(resource_id)
        .execute(&mut conn)
        .unwrap();
    }

    assert_eq!(
        app.state
            .data_resource_service
            .revalidate_user_aliases(&user.id)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        app.state
            .data_resource_service
            .get(&user, resource_id)
            .await
            .unwrap()
            .status,
        "needs_rename"
    );
    assert_eq!(
        app.state
            .data_resource_service
            .rename_alias(&user, resource_id, "safe_again")
            .await
            .unwrap()
            .status,
        "live"
    );
}

#[tokio::test]
async fn repeated_default_alias_gets_a_stable_server_fallback() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "alias-fallback").await;
    let mut aliases = Vec::new();
    for suffix in ["first", "second"] {
        let batch = app
            .state
            .upload_batch_service
            .accept(
                &user,
                session_id,
                format!("alias-{suffix}"),
                UploadSourceKind::Upload,
                vec![csv(&app, suffix, "profile.csv", b"x\n1\n")],
            )
            .await
            .unwrap();
        let candidate = &batch.candidates[0];
        let published = app
            .state
            .data_resource_service
            .publish_candidates(
                &user,
                batch.id.parse().unwrap(),
                vec![CandidateDecision {
                    candidate_id: candidate.id.clone(),
                    purpose: CandidatePurpose::DataResource,
                    selected: true,
                    table_name: None,
                    display_name: None,
                    alias: candidate.proposed_alias.clone(),
                }],
            )
            .await
            .unwrap();
        aliases.push(published.resources[0].alias.clone());
    }
    assert_eq!(aliases[0], "profile");
    assert!(aliases[1].starts_with("data_"));
    assert_ne!(aliases[0], aliases[1]);
}

#[tokio::test]
async fn para_contract_cannot_be_reclassified_as_a_renamed_model_table() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "para-purpose").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "para-purpose".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "master".into(),
                file_name: "master.dss".into(),
                relative_path: Some("master.dss".into()),
                media_type: None,
                staged: app
                    .state
                    .artifact_service
                    .store()
                    .stage(Cursor::new(b"New Circuit.demo\n"), StageLimits::default())
                    .unwrap(),
            }],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| {
            let is_para = candidate.inferred_schema_ref.as_deref() == Some("frames.para");
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if is_para {
                    CandidatePurpose::ModelTable
                } else {
                    CandidatePurpose::Skip
                },
                selected: is_para,
                table_name: is_para.then(|| "renamed_para".into()),
                display_name: None,
                alias: None,
            }
        })
        .collect();

    let error = app
        .state
        .data_resource_service
        .publish_candidates(&user, batch.id.parse().unwrap(), decisions)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "upload_candidate_invalid");
    assert!(
        app.state
            .version_service
            .workspace(&user, session_id)
            .await
            .unwrap()
            .draft
            .is_none()
    );
}

#[tokio::test]
async fn unresolved_upload_cannot_be_confirmed_or_lose_its_reservation() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "unresolved-confirm").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "unresolved-confirm".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "ambiguous", "ambiguous.csv", b"a;b\n1;2\n")],
        )
        .await
        .unwrap();
    assert_ne!(
        batch.status,
        zjudss::application::UploadBatchStatus::ReadyToConfirm
    );
    let reserved = batch.reserved_bytes;

    let error = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            batch.id.parse().unwrap(),
            vec![],
            None,
            None,
            Some("must-not-claim"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "upload_state_invalid");
    let after = app
        .state
        .upload_batch_service
        .get(&user, batch.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(after.status, batch.status);
    assert_eq!(after.reserved_bytes, reserved);
}

#[tokio::test]
async fn partial_confirmation_releases_completed_files_and_freezes_published_decisions() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "partial-settlement").await;
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "partial-settlement".into(),
            UploadSourceKind::Upload,
            vec![
                csv(
                    &app,
                    "fuse",
                    "fuse.csv",
                    b"name,monitored_obj,switched_obj,fuse_curve\nf1,line.l1,line.l1,tcc1\n",
                ),
                csv(&app, "forecast", "forecast.csv", b"time,value\n0,1\n"),
                csv(&app, "spare", "spare.csv", b"time,value\n0,2\n"),
            ],
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let is_model = candidate.inferred_schema_ref.as_deref() == Some("frames.fuse");
            let is_spare = index == 2;
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if is_spare {
                    CandidatePurpose::Skip
                } else if is_model {
                    CandidatePurpose::ModelTable
                } else {
                    CandidatePurpose::DataResource
                },
                selected: !is_spare,
                table_name: is_model.then(|| "fuse".into()),
                display_name: None,
                alias: (!is_model).then(|| "forecast".into()),
            }
        })
        .collect::<Vec<_>>();
    let first = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            batch.id.parse().unwrap(),
            decisions.clone(),
            None,
            Some(DraftRef {
                id: i32::MAX,
                revision: 1,
            }),
            Some("partial-settlement"),
        )
        .await
        .unwrap();
    assert_eq!(first.status, "partially_confirmed");
    assert_eq!(first.model_status, "failed");
    assert_eq!(first.resource_status, "published");
    let partial = app
        .state
        .upload_batch_service
        .get(&user, batch.id.parse().unwrap())
        .await
        .unwrap();
    assert!(partial.reserved_bytes > 0);
    assert!(partial.reserved_bytes < batch.reserved_bytes);

    let mut changed = decisions;
    let skipped = changed
        .iter_mut()
        .find(|decision| decision.purpose == CandidatePurpose::Skip)
        .unwrap();
    skipped.purpose = CandidatePurpose::DataResource;
    skipped.selected = true;
    skipped.alias = Some("late_resource".into());
    let error = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            batch.id.parse().unwrap(),
            changed,
            None,
            None,
            Some("partial-settlement"),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "upload_candidate_invalid");
}

#[tokio::test]
async fn discarding_a_failed_resource_publish_releases_original_and_normalized_artifacts() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = session(&app, &user, "failed-resource-cleanup").await;
    let first = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "cleanup-first".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "first", "first.csv", b"x\n1\n")],
        )
        .await
        .unwrap();
    app.state
        .data_resource_service
        .publish_candidates(
            &user,
            first.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: first.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("occupied".into()),
            }],
        )
        .await
        .unwrap();
    let baseline = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    let failed = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session_id,
            "cleanup-failed".into(),
            UploadSourceKind::Upload,
            vec![csv(&app, "failed", "failed.csv", b"x\n2\n")],
        )
        .await
        .unwrap();
    let confirmation = app
        .state
        .data_resource_service
        .publish_candidates(
            &user,
            failed.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: failed.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("occupied".into()),
            }],
        )
        .await
        .unwrap();
    assert_eq!(confirmation.resource_status, "failed");
    app.state
        .upload_batch_service
        .discard(&user, failed.id.parse().unwrap())
        .await
        .unwrap();
    let after = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert_eq!(after.used_artifacts, baseline.used_artifacts);
    assert_eq!(after.used_bytes, baseline.used_bytes);
}
