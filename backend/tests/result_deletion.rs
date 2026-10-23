mod common;

use std::io::Cursor;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{
    CreateScriptRequest, PublishDraftRequest, ScriptSource, UploadBatch, UploadFile,
};
use backend::storage::StageLimits;
use common::TestApp;
use diesel::prelude::*;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn create_completed_run(app: &TestApp) -> (backend::auth::CurrentUser, String, i32, i32) {
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "results", 1)
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
            "result-model".into(),
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
                .into_iter()
                .map(|candidate| candidate.id)
                .collect(),
        )
        .await
        .unwrap();
    let version = app
        .state
        .version_service
        .publish(
            &user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "result-version".into(),
            },
        )
        .await
        .unwrap();
    let script = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "result.rss".into(),
                content: "result = bus".into(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
            },
        )
        .await
        .unwrap();
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/scripts/{}/runs", script.id))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "modelVersionId": version.version.version.id.to_string(),
                        "clientRunId": "result-run"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let run_id = body["run"]["id"].as_str().unwrap().parse::<i32>().unwrap();
    let dataset_id = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let datasets = app.state.run_service.datasets(&user, run_id).await.unwrap();
            if let Some(dataset) = datasets.first() {
                break dataset.id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    (user, cookie, run_id, dataset_id)
}

async fn request(
    app: &TestApp,
    cookie: &str,
    method: &str,
    path: String,
) -> axum::response::Response {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", cookie)
        .body(Body::empty())
        .unwrap();
    app.router.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn deleting_one_result_hides_content_releases_quota_and_keeps_run() {
    let app = TestApp::spawn().await;
    let (user, cookie, run_id, dataset_id) = create_completed_run(&app).await;
    let before = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();

    let deleted = request(
        &app,
        &cookie,
        "DELETE",
        format!("/api/datasets/{dataset_id}"),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let payload: Value =
        serde_json::from_slice(&to_bytes(deleted.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(payload["datasetId"], dataset_id.to_string());
    assert!(payload["tombstone"]["deletedAt"].is_string());

    for suffix in ["", "/rows", "/download"] {
        let response = request(
            &app,
            &cookie,
            "GET",
            format!("/api/datasets/{dataset_id}{suffix}"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    assert!(
        app.state
            .run_service
            .datasets(&user, run_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        app.state.run_service.find(&user, run_id).await.unwrap().id,
        run_id
    );
    let after = app
        .state
        .artifact_service
        .get_usage(user.storage_scope_id)
        .await
        .unwrap();
    assert!(after.used_artifacts < before.used_artifacts);
    assert!(after.used_bytes < before.used_bytes);

    let messages = app
        .state
        .message_repo
        .list_by_session(
            &user.id,
            app.state
                .run_service
                .find(&user, run_id)
                .await
                .unwrap()
                .session_id,
        )
        .await
        .unwrap();
    assert!(messages.iter().any(|message| message.role == "assistant"
        && message.content.contains("结果数据集")
        && message.content.contains("已删除")));
}

#[tokio::test]
async fn busy_results_are_protected_and_materialized_descendants_keep_tombstone_lineage() {
    let app = TestApp::spawn().await;
    let (user, cookie, run_id, source_id) = create_completed_run(&app).await;
    let run = app.state.run_service.find(&user, run_id).await.unwrap();
    let source = app
        .state
        .dataset_service
        .metadata(&user, source_id)
        .await
        .unwrap();

    let analysis_id = {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query("INSERT INTO result_analysis_runs(session_id,intent_json,script_artifact_id,status,stage,agent_bundle_version,agent_skill_refs_json,engine_contract_version,created_at_ms) SELECT session_id,'{}',script_artifact_id,'pending','queued','test','[]',engine_contract_version,created_at_ms FROM simulation_runs WHERE id=?1")
            .bind::<diesel::sql_types::Integer,_>(run_id)
            .execute(&mut conn).unwrap();
        let id: i32 = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
            "last_insert_rowid()",
        ))
        .get_result(&mut conn)
        .unwrap();
        diesel::sql_query("INSERT INTO result_analysis_inputs(analysis_run_id,dataset_id,alias,ordinal,source_name,source_schema_json,source_schema_hash,source_producer_json) VALUES(?1,?2,'source',0,?3,?4,?5,?6)")
            .bind::<diesel::sql_types::Integer,_>(id)
            .bind::<diesel::sql_types::Integer,_>(source_id)
            .bind::<diesel::sql_types::Text,_>(&source.name)
            .bind::<diesel::sql_types::Text,_>(serde_json::to_string(&source.columns).unwrap())
            .bind::<diesel::sql_types::Text,_>(&source.schema_hash)
            .bind::<diesel::sql_types::Text,_>(json!({"runId":run_id.to_string()}).to_string())
            .execute(&mut conn).unwrap();
        id
    };
    let busy = request(
        &app,
        &cookie,
        "DELETE",
        format!("/api/datasets/{source_id}"),
    )
    .await;
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    let busy_body: Value =
        serde_json::from_slice(&to_bytes(busy.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(busy_body["code"], "result_busy");

    let derived_id = {
        let mut conn = app.pool.get().unwrap();
        diesel::sql_query("UPDATE result_analysis_runs SET status='succeeded',stage='present',finished_at_ms=created_at_ms WHERE id=?1")
            .bind::<diesel::sql_types::Integer,_>(analysis_id)
            .execute(&mut conn).unwrap();
        diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+1 WHERE storage_scope_id=?1 AND artifact_id=(SELECT artifact_id FROM result_datasets WHERE id=?2)")
            .bind::<diesel::sql_types::Integer,_>(user.storage_scope_id)
            .bind::<diesel::sql_types::Integer,_>(source_id)
            .execute(&mut conn).unwrap();
        diesel::sql_query("INSERT INTO result_datasets(analysis_run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) SELECT ?1,artifact_id,'derived',schema_json,schema_hash,row_count,size_bytes,created_at_ms FROM result_datasets WHERE id=?2")
            .bind::<diesel::sql_types::Integer,_>(analysis_id)
            .bind::<diesel::sql_types::Integer,_>(source_id)
            .execute(&mut conn).unwrap();
        let derived: i32 = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
            "last_insert_rowid()",
        ))
        .get_result(&mut conn)
        .unwrap();
        diesel::sql_query("INSERT INTO dataset_lineage_snapshots(derived_dataset_id,source_dataset_id,source_name,source_schema_json,source_schema_hash,source_producer_json,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,1)")
            .bind::<diesel::sql_types::Integer,_>(derived)
            .bind::<diesel::sql_types::Integer,_>(source_id)
            .bind::<diesel::sql_types::Text,_>(&source.name)
            .bind::<diesel::sql_types::Text,_>(serde_json::to_string(&source.columns).unwrap())
            .bind::<diesel::sql_types::Text,_>(&source.schema_hash)
            .bind::<diesel::sql_types::Text,_>(json!({"runId":run_id.to_string()}).to_string())
            .execute(&mut conn).unwrap();
        diesel::sql_query("INSERT INTO dataset_views(source_dataset_id,view_dataset_id,name,kind,spec_json,is_default,created_at_ms) VALUES(?1,?2,'materialized','line','{\"kind\":\"line\",\"x\":\"id\",\"y\":[\"value\"]}',1,1)")
            .bind::<diesel::sql_types::Integer,_>(source_id)
            .bind::<diesel::sql_types::Integer,_>(derived)
            .execute(&mut conn).unwrap();
        derived
    };

    assert_eq!(
        request(
            &app,
            &cookie,
            "DELETE",
            format!("/api/datasets/{source_id}")
        )
        .await
        .status(),
        StatusCode::OK
    );
    let derived = request(&app, &cookie, "GET", format!("/api/datasets/{derived_id}")).await;
    assert_eq!(derived.status(), StatusCode::OK);
    let derived: Value =
        serde_json::from_slice(&to_bytes(derived.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(
        derived["lineage"][0]["sourceDatasetId"],
        source_id.to_string()
    );
    assert_eq!(derived["lineage"][0]["deleted"], true);
    assert_eq!(derived["lineage"][0]["name"], source.name);

    let view = request(&app, &cookie, "GET", "/api/dataset-views/1/data".into()).await;
    assert_eq!(view.status(), StatusCode::OK);
    let new_analysis = app
        .state
        .analysis_service
        .run(backend::application::AnalysisRequest {
            principal: user,
            session_id: run.session_id,
            request_message_id: None,
            intent: json!({}),
            script: "result = source".into(),
            inputs: vec![backend::application::AnalysisInput {
                dataset_id: source_id,
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            initial_views: Default::default(),
            skill_refs: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(new_analysis.code, "analysis_input_not_found");
}
