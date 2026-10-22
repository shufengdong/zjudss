mod common;
use backend::application::{
    ArtifactMetadata, DataframeBindingDraft, ModelVersionStatus, RunStudyRequest, SourceKind,
    VersionDraft, VersionInputDraft, VersionOrigin,
};
use backend::auth::{AccessScope, Principal, StorageScopeRef};
use backend::storage::StageLimits;
use common::TestApp;
use diesel::prelude::*;
use serde_json::json;
use std::io::Cursor;
fn p() -> Principal {
    Principal {
        access: AccessScope {
            kind: "instance".into(),
            key: "instance".into(),
        },
        storage: StorageScopeRef {
            kind: "instance".into(),
            key: "instance".into(),
        },
        subject: "instance".into(),
    }
}
async fn two_tasks(app: &TestApp) -> (Principal, i32, i32, i32) {
    let p = p();
    let s = app
        .state
        .session_repo
        .create(&p.access, "compare", 1)
        .await
        .unwrap();
    let w = app
        .state
        .workspace_service
        .get_or_create(&p.access, s.id)
        .await
        .unwrap();
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&p.storage)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,p\nb1,1\n"), StageLimits::default())
        .unwrap();
    let a = app
        .state
        .artifact_service
        .commit(
            scope.id,
            staged,
            ArtifactMetadata {
                format: "csv".into(),
                media_type: None,
            },
        )
        .await
        .unwrap();
    let v = app
        .state
        .workspace_service
        .publish_version(
            &p.access,
            w.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({}),
                patch: None,
                inputs: vec![VersionInputDraft {
                    artifact_id: a.id,
                    sha256: a.sha256,
                    role: "input".into(),
                    relative_path: Some("bus.csv".into()),
                    display_name: "bus.csv".into(),
                    source_artifact_id: None,
                    source_artifact_sha256: None,
                    sheet_name: None,
                    binding: Some(DataframeBindingDraft {
                        table_kind: Some("bus".into()),
                        confidence: Some(1.0),
                        resolution: "user_confirmed".into(),
                        columns: json!([]),
                        evidence: json!({}),
                    }),
                }],
            },
        )
        .await
        .unwrap();
    let mut ids = Vec::new();
    for n in [1, 2] {
        ids.push(
            app.state
                .simulation_service
                .run(RunStudyRequest {
                    principal: p.clone(),
                    session_id: s.id,
                    model_version_id: v.id,
                    study_type: "power_flow".into(),
                    study_spec: json!({"case":n}),
                    script: Some(format!("result=bus # {n}")),
                    skill_refs: vec![],
                })
                .await
                .unwrap()
                .task
                .id,
        );
    }
    (p, s.id, ids[0], ids[1])
}
#[tokio::test]
async fn compatible_tasks_create_traceable_analysis_and_difference_dataset() {
    let app = TestApp::spawn().await;
    let (p, session, left, right) = two_tasks(&app).await;
    let c = app
        .state
        .comparison_service
        .compare(
            &p,
            session,
            left,
            right,
            None,
            vec![json!({"id":"comparison"})],
        )
        .await
        .unwrap();
    let (found, compat, datasets) = app.state.comparison_service.find(&p, c.id).await.unwrap();
    assert_eq!(found.analysis_run_id, c.analysis_run_id);
    assert_eq!(compat["stableKeys"], json!(["id"]));
    assert_eq!(datasets.len(), 1);
    let meta = app
        .state
        .dataset_service
        .metadata(&p, datasets[0].id)
        .await
        .unwrap();
    assert_eq!(
        meta.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "key",
            "baseline",
            "candidate",
            "absolute",
            "relative",
            "diagnostic"
        ]
    );
    let page = app
        .state
        .dataset_service
        .page(&p, datasets[0].id, None, None, &[])
        .await
        .unwrap();
    assert_eq!(page.rows[0][4], serde_json::Value::Null);
    let msgs = app
        .state
        .message_repo
        .list_by_session(&p.access, session)
        .await
        .unwrap();
    assert!(msgs.iter().any(|m| m.kind == "comparison"));
}
#[tokio::test]
async fn same_task_foreign_session_and_missing_stable_key_fail_without_comparison() {
    let app = TestApp::spawn().await;
    let (p, session, left, right) = two_tasks(&app).await;
    let e = app
        .state
        .comparison_service
        .compare(&p, session, left, left, None, vec![])
        .await
        .unwrap_err();
    assert_eq!(e.code, "comparison_incompatible");
    let e = app
        .state
        .comparison_service
        .compare(&p, session + 999, left, right, None, vec![])
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found");
    let datasets = app.state.dataset_service.list(&p, left).await.unwrap();
    let mut conn = app.pool.get().unwrap();
    diesel::update(
        backend::db::schema::result_datasets::table.find(datasets[0].id.parse::<i32>().unwrap()),
    )
    .set(
        backend::db::schema::result_datasets::schema_json
            .eq(json!([{"name":"value","dtype":"Int64","nullable":false,"unit":null}]).to_string()),
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);
    let e = app
        .state
        .comparison_service
        .compare(&p, session, left, right, None, vec![])
        .await
        .unwrap_err();
    assert_eq!(e.code, "comparison_incompatible");
    let mut conn = app.pool.get().unwrap();
    let count: i64 = backend::db::schema::task_comparisons::table
        .count()
        .get_result(&mut conn)
        .unwrap();
    assert_eq!(count, 0);
}
