mod common;

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use parquet::arrow::ArrowWriter;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::application::{ArtifactMetadata, now_ms};
use zjudss::storage::StageLimits;

fn presentation(line: bool) -> Value {
    let mut value = json!({
        "legend":{"show":true,"position":"top","orientation":"horizontal"},
        "palette":"colorblind","title":{"show":true},"axis":{"showGrid":true}
    });
    if line {
        value["line"] = json!({"width":2,"style":"solid","showSymbol":false});
    }
    value
}

fn line_config() -> Value {
    json!({
        "version":1,"kind":"line","presentation":presentation(true),
        "x":{"column":"bus"},"xAxisMode":"category",
        "measures":[{"column":"load"}],"order":"source",
        "aggregation":"none","sampling":{"mode":"none"}
    })
}

fn six_configs() -> Vec<Value> {
    vec![
        line_config(),
        json!({"version":1,"kind":"bar","presentation":presentation(false),"x":{"column":"bus"},"measures":[{"column":"load"}],"order":"source","aggregation":"none"}),
        json!({"version":1,"kind":"scatter","presentation":presentation(false),"x":{"column":"load"},"measures":[{"column":"aux"}]}),
        json!({"version":1,"kind":"heatmap","presentation":presentation(false),"x":{"column":"bus"},"y":{"column":"group"},"value":{"column":"load"},"aggregation":"mean"}),
        json!({"version":1,"kind":"boxplot","presentation":presentation(false),"category":{"column":"group"},"value":{"column":"load"},"showOutliers":true}),
        json!({"version":1,"kind":"histogram","presentation":presentation(false),"value":{"column":"load"},"bins":3}),
    ]
}

fn parquet_bytes() -> Vec<u8> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("bus", DataType::Utf8, false),
        Field::new("group", DataType::Utf8, false),
        Field::new("load", DataType::Float64, false),
        Field::new("aux", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["A", "B", "C", "D"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["G1", "G1", "G2", "G2"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![1., 2., 3., 4.])) as ArrayRef,
            Arc::new(Float64Array::from(vec![4., 3., 2., 1.])) as ArrayRef,
        ],
    )
    .unwrap();
    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = ArrowWriter::try_new(&mut output, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
    }
    output.into_inner()
}

async fn seed_dataset(app: &TestApp) -> (String, String, i32, i32) {
    let user = common::test_user("alice");
    let session = app
        .state
        .session_repo
        .create(&user.id, "charts", now_ms())
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(parquet_bytes()), StageLimits::default())
        .unwrap();
    let artifact = app
        .state
        .artifact_service
        .commit(
            user.storage_scope_id,
            staged,
            ArtifactMetadata {
                format: "parquet".into(),
                media_type: Some("application/vnd.apache.parquet".into()),
            },
        )
        .await
        .unwrap();
    let schema = json!([
        {"name":"bus","dtype":"Utf8","nullable":false,"unit":null},
        {"name":"group","dtype":"Utf8","nullable":false,"unit":null},
        {"name":"load","dtype":"Float64","nullable":false,"unit":"MW"},
        {"name":"aux","dtype":"Float64","nullable":false,"unit":"MW"}
    ])
    .to_string();
    let mut connection = app.pool.get().unwrap();
    connection
        .batch_execute(&format!(
            "INSERT INTO simulation_runs(session_id,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,run_kind,created_at_ms) VALUES({},'chart-run','succeeded','completed','test','[]','managed',1)",
            session.id
        ))
        .unwrap();
    let run_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result::<i32>(&mut connection)
    .unwrap();
    diesel::sql_query("INSERT INTO result_datasets(run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) VALUES(?1,?2,'power',?3,?4,4,?5,1)")
        .bind::<diesel::sql_types::Integer, _>(run_id)
        .bind::<diesel::sql_types::Integer, _>(artifact.id)
        .bind::<diesel::sql_types::Text, _>(schema)
        .bind::<diesel::sql_types::Text, _>("a".repeat(64))
        .bind::<diesel::sql_types::BigInt, _>(artifact.size_bytes)
        .execute(&mut connection)
        .unwrap();
    let dataset_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result::<i32>(&mut connection)
    .unwrap();
    (
        app.login_cookie("alice").await,
        app.login_cookie("bob").await,
        session.id,
        dataset_id,
    )
}

async fn request(
    app: &TestApp,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    json_body: Option<Value>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    let body = match json_body {
        Some(value) => {
            request = request.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    app.router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn body(response: axum::response::Response) -> Value {
    serde_json::from_slice(
        &to_bytes(response.into_body(), 10 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn chart_routes_compile_persist_replay_survive_source_delete_and_enforce_scope() {
    let app = TestApp::spawn().await;
    let (alice, bob, session_id, dataset_id) = seed_dataset(&app).await;
    let capabilities_path = format!("/api/datasets/{dataset_id}/chart-capabilities");
    assert_eq!(
        request(&app, "GET", &capabilities_path, None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let capabilities = request(&app, "GET", &capabilities_path, Some(&alice), None).await;
    assert_eq!(capabilities.status(), StatusCode::OK);
    let capabilities = body(capabilities).await;
    assert_eq!(capabilities["datasetId"], dataset_id.to_string());
    assert_eq!(capabilities["fields"].as_array().unwrap().len(), 4);

    for config in six_configs() {
        let kind = config["kind"].clone();
        let preview = request(
            &app,
            "POST",
            &format!("/api/datasets/{dataset_id}/chart-previews"),
            Some(&alice),
            Some(json!({"config":config})),
        )
        .await;
        assert_eq!(preview.status(), StatusCode::OK);
        let preview = body(preview).await;
        assert_eq!(preview["kind"], kind);
        assert!(preview["viewId"].is_null());
    }
    let count = diesel::select(diesel::dsl::sql::<diesel::sql_types::BigInt>(
        "count(*) FROM dataset_views",
    ))
    .get_result::<i64>(&mut app.pool.get().unwrap())
    .unwrap();
    assert_eq!(count, 0);

    let create = json!({
        "clientOperationId":"1f6d0000-0000-4000-8000-000000000001",
        "name":"Bus load","config":line_config()
    });
    let created = request(
        &app,
        "POST",
        &format!("/api/datasets/{dataset_id}/views"),
        Some(&alice),
        Some(create.clone()),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = body(created).await;
    let view_id = created["view"]["id"].as_str().unwrap().to_string();
    let mut saved_views = vec![("line".to_string(), view_id.clone())];
    assert_eq!(created["replayed"], false);
    assert_eq!(created["view"]["contractVersion"], "chart-model-v1");
    for (index, config) in six_configs().into_iter().skip(1).enumerate() {
        let kind = config["kind"].as_str().unwrap().to_string();
        let created = request(
            &app,
            "POST",
            &format!("/api/datasets/{dataset_id}/views"),
            Some(&alice),
            Some(json!({
                "clientOperationId":format!("3f6d0000-0000-4000-8000-00000000000{}", index + 2),
                "name":format!("saved {kind}"),
                "config":config,
            })),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED, "save {kind}");
        let created = body(created).await;
        saved_views.push((kind, created["view"]["id"].as_str().unwrap().to_string()));
    }
    let replay = request(
        &app,
        "POST",
        &format!("/api/datasets/{dataset_id}/views"),
        Some(&alice),
        Some(create.clone()),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(body(replay).await["replayed"], true);

    for path in [
        format!("/api/dataset-views/{view_id}"),
        format!("/api/dataset-views/{view_id}/data"),
        format!("/api/sessions/{session_id}/dataset-views?limit=1"),
    ] {
        let response = request(&app, "GET", &path, Some(&alice), None).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
    let model = body(
        request(
            &app,
            "GET",
            &format!("/api/dataset-views/{view_id}/data"),
            Some(&alice),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(model["viewId"], view_id);
    assert_eq!(model["name"], "Bus load");

    for (method, path, request_body) in [
        ("GET", capabilities_path, None),
        (
            "POST",
            format!("/api/datasets/{dataset_id}/chart-previews"),
            Some(json!({"config":line_config()})),
        ),
        (
            "POST",
            format!("/api/datasets/{dataset_id}/views"),
            Some(
                json!({"clientOperationId":"2f6d0000-0000-4000-8000-000000000002","name":"foreign","config":line_config()}),
            ),
        ),
        ("GET", format!("/api/dataset-views/{view_id}"), None),
        ("GET", format!("/api/dataset-views/{view_id}/data"), None),
        ("DELETE", format!("/api/dataset-views/{view_id}"), None),
        (
            "GET",
            format!("/api/sessions/{session_id}/dataset-views"),
            None,
        ),
    ] {
        assert_eq!(
            request(&app, method, &path, Some(&bob), request_body)
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{method} {path}"
        );
    }

    assert_eq!(
        request(
            &app,
            "DELETE",
            &format!("/api/datasets/{dataset_id}"),
            Some(&alice),
            None,
        )
        .await
        .status(),
        StatusCode::OK
    );
    for (kind, saved_view_id) in &saved_views {
        let persisted = body(
            request(
                &app,
                "GET",
                &format!("/api/dataset-views/{saved_view_id}/data"),
                Some(&alice),
                None,
            )
            .await,
        )
        .await;
        assert_eq!(persisted["kind"], *kind);
        assert_eq!(persisted["sourceDeleted"], true);
    }
    let replay_after_source_delete = request(
        &app,
        "POST",
        &format!("/api/datasets/{dataset_id}/views"),
        Some(&alice),
        Some(create),
    )
    .await;
    assert_eq!(replay_after_source_delete.status(), StatusCode::OK);
    assert_eq!(body(replay_after_source_delete).await["replayed"], true);
    let deleted = request(
        &app,
        "DELETE",
        &format!("/api/dataset-views/{view_id}"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    let deleted = body(deleted).await;
    assert_eq!(deleted["viewId"], view_id);
    assert_eq!(deleted["tombstone"]["name"], "Bus load");
    let operation_gone = request(
        &app,
        "POST",
        &format!("/api/datasets/{dataset_id}/views"),
        Some(&alice),
        Some(json!({
            "clientOperationId":"1f6d0000-0000-4000-8000-000000000001",
            "name":"Changed after deletion","config":line_config()
        })),
    )
    .await;
    assert_eq!(operation_gone.status(), StatusCode::CONFLICT);
    assert_eq!(body(operation_gone).await["code"], "chart_operation_gone");
}

#[tokio::test]
async fn chart_errors_have_stable_status_and_code() {
    let app = TestApp::spawn().await;
    let (alice, _, session_id, dataset_id) = seed_dataset(&app).await;
    let invalid = request(
        &app,
        "POST",
        &format!("/api/datasets/{dataset_id}/chart-previews"),
        Some(&alice),
        Some(json!({"config":{"version":1,"kind":"histogram","presentation":presentation(false),"value":{"column":"missing"},"bins":10}})),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(invalid).await["code"], "chart_column_missing");
    let malformed = request(
        &app,
        "POST",
        &format!("/api/datasets/{dataset_id}/chart-previews"),
        Some(&alice),
        Some(json!({"config":{"version":1,"kind":"histogram","presentation":presentation(false),"value":{"column":"load"},"bins":10,"arbitraryEchartsOption":{}}})),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(malformed).await["code"], "chart_config_invalid");
    let cursor = request(
        &app,
        "GET",
        &format!("/api/sessions/{session_id}/dataset-views?limit=0"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(cursor.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(cursor).await["code"], "chart_cursor_invalid");
    let malformed_query = request(
        &app,
        "GET",
        &format!("/api/sessions/{session_id}/dataset-views?limit=not-a-number"),
        Some(&alice),
        None,
    )
    .await;
    assert_eq!(malformed_query.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(malformed_query).await["code"], "chart_cursor_invalid");
}
