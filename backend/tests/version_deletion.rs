mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use diesel::prelude::*;
use serde_json::{Value, json};
use std::io::Cursor;
use tower::ServiceExt;
use zjudss::application::{UploadBatch, UploadFile};
use zjudss::storage::StageLimits;

async fn published_version(app: &TestApp, cookie: &str) -> (i32, i32) {
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "delete", 1)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,name\n1,bus\n"), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            &user,
            session.id,
            "delete-model".into(),
            None,
            UploadBatch {
                client_batch_id: "unused".into(),
                files: vec![UploadFile {
                    client_file_id: "bus".into(),
                    file_name: "bus.csv".into(),
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
    let draft = app
        .state
        .model_import_service
        .resolve_and_apply(
            &user,
            accepted.id.parse().unwrap(),
            None,
            ready
                .candidates
                .iter()
                .map(|item| item.id.clone())
                .collect(),
        )
        .await
        .unwrap();
    let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/versions", session.id)).header("cookie", cookie).header("content-type", "application/json").body(Body::from(json!({"expectedDraftId":draft.id,"expectedDraftRevision":draft.revision,"clientPublishId":"delete-publish"}).to_string())).unwrap()).await.unwrap();
    assert!(response.status().is_success());
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    (
        session.id,
        body["version"]["id"].as_str().unwrap().parse().unwrap(),
    )
}

async fn preview(app: &TestApp, cookie: &str, version_id: i32) -> axum::response::Response {
    app.router
        .clone()
        .oneshot(
            Request::post(format!("/api/versions/{version_id}/deletion-preview"))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn deletion_requires_preview_rejects_active_and_detects_changed_impact() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice@example.com").await;
    let (_session_id, version_id) = published_version(&app, &cookie).await;
    assert_eq!(
        preview(&app, &cookie, version_id).await.status(),
        StatusCode::CONFLICT
    );

    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("UPDATE workspaces SET active_version_id=NULL WHERE active_version_id=?1")
        .bind::<diesel::sql_types::Integer, _>(version_id)
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let response = preview(&app, &cookie, version_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let token = value["impactToken"].as_str().unwrap();

    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("INSERT INTO script_resources(session_id,artifact_id,name,source_kind,content_hash,validation_status,required_aliases_json,outputs_json,diagnostics_json,agent_skill_refs_json,created_at_ms) SELECT w.session_id,md.artifact_id,'late.rss','upload',a.sha256,'valid','[]','[]','{}','[]',1 FROM model_versions v JOIN workspaces w ON w.id=v.workspace_id JOIN model_dataframes md ON md.version_id=v.id JOIN artifacts a ON a.id=md.artifact_id WHERE v.id=?1 LIMIT 1").bind::<diesel::sql_types::Integer,_>(version_id).execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO simulation_runs(session_id,model_version_id,model_content_hash,script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,created_at_ms) SELECT s.session_id,v.id,v.content_hash,s.id,s.artifact_id,s.content_hash,'late-run','succeeded','completed','test','[]',2 FROM model_versions v JOIN script_resources s WHERE v.id=?1 ORDER BY s.id DESC LIMIT 1").bind::<diesel::sql_types::Integer,_>(version_id).execute(&mut conn).unwrap();
    drop(conn);
    let response = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/versions/{version_id}?impactToken={token}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn deletion_tombstones_version_runs_and_raw_results_but_keeps_derived_lineage() {
    let app = TestApp::spawn().await;
    let cookie = app.login_cookie("alice@example.com").await;
    let (session_id, version_id) = published_version(&app, &cookie).await;
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("UPDATE workspaces SET active_version_id=NULL WHERE active_version_id=?1")
        .bind::<diesel::sql_types::Integer, _>(version_id)
        .execute(&mut conn)
        .unwrap();
    diesel::sql_query("INSERT INTO script_resources(session_id,artifact_id,name,source_kind,content_hash,validation_status,required_aliases_json,outputs_json,diagnostics_json,agent_skill_refs_json,created_at_ms) SELECT ?1,md.artifact_id,'run.rss','upload',a.sha256,'valid','[]','[]','{}','[]',2 FROM model_dataframes md JOIN artifacts a ON a.id=md.artifact_id WHERE md.version_id=?2 LIMIT 1").bind::<diesel::sql_types::Integer,_>(session_id).bind::<diesel::sql_types::Integer,_>(version_id).execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO simulation_runs(session_id,model_version_id,model_content_hash,script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,created_at_ms) SELECT ?1,v.id,v.content_hash,s.id,s.artifact_id,s.content_hash,'run','succeeded','completed','test','[]',3 FROM model_versions v JOIN script_resources s ON s.session_id=?1 WHERE v.id=?2 ORDER BY s.id DESC LIMIT 1").bind::<diesel::sql_types::Integer,_>(session_id).bind::<diesel::sql_types::Integer,_>(version_id).execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO result_datasets(run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) SELECT r.id,r.script_artifact_id,'raw','{}',r.script_content_hash,1,1,4 FROM simulation_runs r WHERE r.client_run_id='run'").execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO result_analysis_runs(session_id,intent_json,script_artifact_id,status,stage,agent_bundle_version,agent_skill_refs_json,engine_contract_version,created_at_ms) SELECT ?1,'{}',artifact_id,'succeeded','completed','test','[]','test',5 FROM script_resources WHERE session_id=?1 LIMIT 1").bind::<diesel::sql_types::Integer,_>(session_id).execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO result_datasets(analysis_run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) SELECT ar.id,s.artifact_id,'derived','{}',s.content_hash,1,1,6 FROM result_analysis_runs ar JOIN script_resources s ON s.session_id=ar.session_id WHERE ar.session_id=?1 LIMIT 1").bind::<diesel::sql_types::Integer,_>(session_id).execute(&mut conn).unwrap();
    diesel::sql_query("INSERT INTO dataset_lineage_snapshots(derived_dataset_id,source_dataset_id,source_name,source_schema_json,source_schema_hash,source_producer_json,created_at_ms) SELECT d.id,r.id,r.result_name,r.schema_json,r.schema_hash,'{}',7 FROM result_datasets d JOIN result_datasets r WHERE d.analysis_run_id IS NOT NULL AND r.run_id IS NOT NULL LIMIT 1").execute(&mut conn).unwrap();
    drop(conn);

    let response = preview(&app, &cookie, version_id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(value["runCount"], 1);
    assert_eq!(value["resultCount"], 1);
    let token = value["impactToken"].as_str().unwrap();
    let response = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/versions/{version_id}?impactToken={token}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    #[derive(QueryableByName)]
    struct Counts {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        versions: i64,
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        runs: i64,
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        raw: i64,
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        derived: i64,
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        lineage: i64,
    }
    let mut conn = app.pool.get().unwrap();
    let counts = diesel::sql_query("SELECT (SELECT count(*) FROM model_versions WHERE id=?1 AND deleted_at_ms IS NOT NULL) versions,(SELECT count(*) FROM simulation_runs WHERE model_version_id=?1 AND deleted_at_ms IS NOT NULL) runs,(SELECT count(*) FROM result_datasets d JOIN simulation_runs r ON r.id=d.run_id WHERE r.model_version_id=?1 AND d.deleted_at_ms IS NOT NULL AND d.artifact_id IS NULL) raw,(SELECT count(*) FROM result_datasets WHERE analysis_run_id IS NOT NULL AND deleted_at_ms IS NULL) derived,(SELECT count(*) FROM dataset_lineage_snapshots) lineage").bind::<diesel::sql_types::Integer,_>(version_id).get_result::<Counts>(&mut conn).unwrap();
    assert_eq!((counts.versions, counts.runs, counts.raw), (1, 1, 1));
    assert_eq!((counts.derived, counts.lineage), (1, 1));
}
