mod common;

use std::{collections::HashMap, io::Cursor};

use backend::{
    application::{
        AnalysisInput, AnalysisRequest, ArtifactMetadata, DataframeBindingDraft,
        ModelVersionStatus, RunStudyRequest, SourceKind, VersionDraft, VersionInputDraft,
        VersionOrigin,
    },
    auth::{CurrentUser, UserId},
    storage::StageLimits,
};
use common::TestApp;
use diesel::{RunQueryDsl, sql_query, sql_types::BigInt};
use serde_json::json;

fn principal() -> CurrentUser {
    CurrentUser {
        id: UserId("instance".into()),
        name: "Instance".into(),
        storage_scope_id: 1,
        auth_session_hash: "test".into(),
    }
}

#[tokio::test]
async fn dataframe_modify_run_analyze_rerun_compare_and_refresh_journey() {
    let app = TestApp::spawn().await;
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.id, "v2 e2e", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&p.id, session.id)
        .await
        .unwrap();
    let scope = app
        .state
        .artifact_service
        .resolve_scope(p.storage_scope_id)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,p\nb1,100\n"), StageLimits::default())
        .unwrap();
    let artifact = app
        .state
        .artifact_service
        .commit(
            scope.id,
            staged,
            ArtifactMetadata {
                format: "csv".into(),
                media_type: Some("text/csv".into()),
            },
        )
        .await
        .unwrap();

    let input = |display: &str| VersionInputDraft {
        artifact_id: artifact.id,
        sha256: artifact.sha256.clone(),
        role: "input".into(),
        relative_path: Some("bus.csv".into()),
        display_name: display.into(),
        source_artifact_id: None,
        source_artifact_sha256: None,
        sheet_name: None,
        binding: Some(DataframeBindingDraft {
            table_kind: Some("bus".into()),
            confidence: Some(1.0),
            resolution: "user_confirmed".into(),
            columns: json!([{"name":"id","dtype":"string"},{"name":"p","dtype":"float64"}]),
            evidence: json!({"source":"header"}),
        }),
    };
    let baseline = app
        .state
        .workspace_service
        .publish_version(
            &p.id,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({"revision":1,"missing":[]}),
                patch: None,
                inputs: vec![input("bus.csv")],
            },
        )
        .await
        .unwrap();
    let candidate = app
        .state
        .workspace_service
        .publish_version(
            &p.id,
            workspace.workspace.id,
            Some(baseline.id),
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Generated,
                manifest: json!({"revision":2,"missing":[]}),
                patch: Some(json!({"operation":"set","column":"p","value":120})),
                inputs: vec![input("bus.csv")],
            },
        )
        .await
        .unwrap();
    app.state
        .workspace_service
        .activate_checked(
            &p.id,
            workspace.workspace.id,
            candidate.id,
            Some(candidate.id),
        )
        .await
        .unwrap();

    let run = |version, case| RunStudyRequest {
        principal: p.clone(),
        session_id: session.id,
        model_version_id: version,
        study_type: "power_flow".into(),
        study_spec: json!({"case":case}),
        script: Some(format!("result = bus # {case}")),
        skill_refs: vec![
            json!({"id":"study-planning"}),
            json!({"id":"rss-generation"}),
        ],
    };
    let before = app
        .state
        .simulation_service
        .run(run(baseline.id, "before"))
        .await
        .unwrap();
    let after = app
        .state
        .simulation_service
        .run(run(candidate.id, "after"))
        .await
        .unwrap();

    let analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: p.clone(),
            session_id: session.id,
            request_message_id: None,
            intent: json!({"question":"show values"}),
            script: "result = source".into(),
            inputs: vec![AnalysisInput {
                dataset_id: after.datasets[0].id,
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            initial_views: HashMap::from([(
                "result".into(),
                vec![(
                    "line".into(),
                    json!({"kind":"line","x":"id","y":["value"],"title":"值"}),
                )],
            )]),
            skill_refs: vec![json!({"id":"result-analysis"})],
        })
        .await
        .unwrap();
    let view = app
        .state
        .dataset_service
        .metadata(&p, analysis.datasets[0].id)
        .await
        .unwrap()
        .views[0]
        .clone();
    assert_eq!(
        app.state
            .dataset_service
            .view_data(&p, view.id.parse().unwrap())
            .await
            .unwrap()
            .rows
            .len(),
        1
    );

    let comparison = app
        .state
        .comparison_service
        .compare(
            &p,
            session.id,
            before.task.id,
            after.task.id,
            None,
            vec![json!({"id":"comparison"})],
        )
        .await
        .unwrap();
    let (_, compatibility, difference) = app
        .state
        .comparison_service
        .find(&p, comparison.id)
        .await
        .unwrap();
    assert_eq!(compatibility["stableKeys"], json!(["id"]));
    assert_eq!(difference.len(), 1);

    // Browser refresh reads all state from persisted repositories rather than process-local UI state.
    let restored_workspace = app
        .state
        .workspace_service
        .get(&p.id, session.id)
        .await
        .unwrap();
    assert_eq!(
        restored_workspace.workspace.active_version_id,
        Some(candidate.id)
    );
    assert_eq!(
        app.state
            .history_service
            .list_simulations(&p, session.id, 50)
            .await
            .unwrap()
            .len(),
        2
    );
    let restored_messages = app
        .state
        .history_service
        .list_messages(&p, session.id, None, None, 100)
        .await
        .unwrap();
    assert!(
        restored_messages
            .iter()
            .any(|message| message.kind == "result")
    );
    assert!(
        restored_messages
            .iter()
            .any(|message| message.kind == "comparison")
    );
}

#[test]
fn release_uses_one_v2_migration_without_v1_result_tables() {
    let migrations = std::fs::read_dir("migrations")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .count();
    assert_eq!(migrations, 1);
    let sql = std::fs::read_to_string("migrations/00000000000000_initial/up.sql").unwrap();
    for removed in ["task_results", "curve_data", "task_files", "params_json"] {
        assert!(!sql.contains(removed), "v1 schema token remains: {removed}");
    }
}

#[tokio::test]
async fn clean_schema_has_no_legacy_tables() {
    let app = TestApp::spawn().await;
    let mut connection = app.pool.get().unwrap();
    let count: i64 = diesel::dsl::sql::<BigInt>(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('task_results','curve_data','task_files')",
    ).get_result(&mut connection).unwrap();
    assert_eq!(count, 0);
    // The FTS release gate is executable on the clean database.
    sql_query("INSERT INTO sessions(title,created_at_ms,updated_at_ms,user_id) VALUES('配电网门禁',1,1,'instance')")
        .execute(&mut connection).unwrap();
    let indexed: i64 = diesel::dsl::sql::<BigInt>(
        "SELECT count(*) FROM chat_search_fts WHERE chat_search_fts MATCH '配电网'",
    )
    .get_result(&mut connection)
    .unwrap();
    assert_eq!(indexed, 1);
}
