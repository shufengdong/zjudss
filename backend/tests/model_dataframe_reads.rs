mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use diesel::prelude::*;
use serde_json::{Value, json};
use tower::ServiceExt;
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;

use common::{CountRow, TestApp};

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

async fn request(app: &TestApp, cookie: &str, path: &str) -> axum::response::Response {
    app.router
        .clone()
        .oneshot(
            Request::get(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn import_table(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    client_id: &str,
    expected: Option<DraftRef>,
    csv: &[u8],
) -> DraftRef {
    common::publish_model_bytes(
        app,
        user,
        session_id,
        client_id,
        "bus.csv",
        csv,
        expected,
        Some("bus"),
    )
    .await
}

async fn workspace(app: &TestApp, cookie: &str, session_id: i32) -> Value {
    let response = request(
        app,
        cookie,
        &format!("/api/sessions/{session_id}/workspace"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

async fn publish(
    app: &TestApp,
    cookie: &str,
    session_id: i32,
    draft: DraftRef,
    client_id: &str,
) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{session_id}/versions"))
                .header("cookie", cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "expectedDraftId": draft.id,
                        "expectedDraftRevision": draft.revision,
                        "clientPublishId": client_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success());
    json_body(response).await["version"].clone()
}

fn database_footprint(app: &TestApp) -> (i64, i64, i64, i64) {
    let mut connection = app.pool.get().unwrap();
    let count = |sql: &str, connection: &mut _| {
        diesel::sql_query(sql)
            .get_result::<CountRow>(connection)
            .unwrap()
            .count
    };
    (
        count("SELECT COUNT(*) AS count FROM artifacts", &mut connection),
        count(
            "SELECT COUNT(*) AS count FROM model_dataframes",
            &mut connection,
        ),
        count(
            "SELECT COUNT(*) AS count FROM draft_dataframes",
            &mut connection,
        ),
        diesel::sql_query(
            "SELECT COALESCE(SUM(ref_count),0) AS count FROM storage_scope_artifacts",
        )
        .get_result::<CountRow>(&mut connection)
        .unwrap()
        .count,
    )
}

#[tokio::test]
async fn version_dataframe_metadata_rows_projection_cursor_and_download_are_read_only() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "read version dataframe", 1)
        .await
        .unwrap();
    let draft = import_table(
        &app,
        &user,
        session.id,
        "version-source",
        None,
        b"id,name\n1,alpha\n2,beta\n3,gamma\n",
    )
    .await;
    let version = publish(&app, &cookie, session.id, draft, "publish-version").await;
    let dataframe_id = version["tables"][0]["id"].as_str().unwrap();
    let version_id = version["id"].as_str().unwrap();
    let before = database_footprint(&app);

    let response = request(
        &app,
        &cookie,
        &format!("/api/model-dataframes/{dataframe_id}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let metadata = json_body(response).await;
    assert_eq!(metadata["id"], dataframe_id);
    assert_eq!(metadata["scope"], "model_version");
    assert_eq!(metadata["ownerId"], version_id);
    assert_eq!(metadata["tableName"], "bus");
    assert_eq!(metadata["rowCount"], 3);
    assert!(metadata["sizeBytes"].as_i64().unwrap() > 0);
    assert!(metadata["schemaHash"].as_str().unwrap().len() == 64);
    assert!(metadata["contentHash"].as_str().unwrap().len() == 64);
    assert_eq!(metadata["columns"].as_array().unwrap().len(), 2);

    let response = request(
        &app,
        &cookie,
        &format!("/api/model-dataframes/{dataframe_id}/rows?limit=2&columns=name"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let first = json_body(response).await;
    assert_eq!(
        first["resourceId"],
        format!("model-dataframe:{dataframe_id}")
    );
    assert_eq!(first["columns"], json!(["name"]));
    assert_eq!(first["rows"], json!([["alpha"], ["beta"]]));
    let cursor = first["nextCursor"].as_str().unwrap();
    let response = request(
        &app,
        &cookie,
        &format!("/api/model-dataframes/{dataframe_id}/rows?limit=2&columns=name&cursor={cursor}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let second = json_body(response).await;
    assert_eq!(second["rows"], json!([["gamma"]]));
    assert!(second["nextCursor"].is_null());

    let response = request(
        &app,
        &cookie,
        &format!("/api/model-dataframes/{dataframe_id}/download"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/vnd.apache.parquet"
    );
    let disposition = response.headers()[header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap();
    assert!(disposition.contains(&format!("model-v{version_id}-bus.parquet")));
    assert!(
        !to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .is_empty()
    );

    assert_eq!(database_footprint(&app), before);
}

#[tokio::test]
async fn historical_version_dataframe_is_readable_without_activation_and_deleted_version_is_not() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "history", 1)
        .await
        .unwrap();
    let first_draft = import_table(
        &app,
        &user,
        session.id,
        "first",
        None,
        b"id,name\n1,first\n",
    )
    .await;
    let first = publish(&app, &cookie, session.id, first_draft, "publish-first").await;
    let first_version = first["id"].as_str().unwrap();
    let first_dataframe = first["tables"][0]["id"].as_str().unwrap();
    let second_draft = import_table(
        &app,
        &user,
        session.id,
        "second",
        None,
        b"id,name\n2,second\n",
    )
    .await;
    let second = publish(&app, &cookie, session.id, second_draft, "publish-second").await;
    let second_version = second["id"].as_str().unwrap();
    assert_ne!(
        first_version, second_version,
        "second model must create a version"
    );
    let activated = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/versions/{second_version}/activate"))
                .header("cookie", &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activated.status(), StatusCode::OK);

    assert_eq!(
        request(
            &app,
            &cookie,
            &format!("/api/model-dataframes/{first_dataframe}/rows")
        )
        .await
        .status(),
        StatusCode::OK
    );

    let preview = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/versions/{first_version}/deletion-preview"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let preview_status = preview.status();
    let preview = json_body(preview).await;
    assert_eq!(preview_status, StatusCode::OK, "{preview}");
    let token = preview["impactToken"].as_str().unwrap().to_string();
    let deleted = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/versions/{first_version}?impactToken={token}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        request(
            &app,
            &cookie,
            &format!("/api/model-dataframes/{first_dataframe}")
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn draft_reads_require_exact_owner_id_and_revision_across_metadata_rows_and_download() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "draft reads", 1)
        .await
        .unwrap();
    let draft = import_table(
        &app,
        &user,
        session.id,
        "draft",
        None,
        b"id,name\n1,alpha\n2,beta\n",
    )
    .await;
    let view = workspace(&app, &cookie, session.id).await;
    let dataframe_id = view["draft"]["tables"][0]["id"].as_str().unwrap();

    let rows = request(
        &app,
        &cookie,
        &format!(
            "/api/draft-dataframes/{dataframe_id}/rows?draftId={}&expectedDraftRevision={}",
            draft.id, draft.revision
        ),
    )
    .await;
    assert_eq!(rows.status(), StatusCode::OK);
    assert_eq!(
        json_body(rows).await["resourceId"],
        format!("draft-dataframe:{dataframe_id}:revision:{}", draft.revision)
    );

    for suffix in ["", "/rows", "/download"] {
        let response = request(
            &app,
            &cookie,
            &format!(
                "/api/draft-dataframes/{dataframe_id}{suffix}?draftId={}&expectedDraftRevision={}",
                draft.id, draft.revision
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{suffix}");
    }
    for suffix in ["", "/rows", "/download"] {
        let response = request(
            &app,
            &cookie,
            &format!(
                "/api/draft-dataframes/{dataframe_id}{suffix}?draftId={}&expectedDraftRevision={}",
                draft.id,
                draft.revision + 1
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{suffix}");
        assert_eq!(json_body(response).await["code"], "draft_revision_conflict");
    }

    publish(&app, &cookie, session.id, draft, "publish-draft").await;
    for suffix in ["", "/rows", "/download"] {
        assert_eq!(
            request(
                &app,
                &cookie,
                &format!(
                    "/api/draft-dataframes/{dataframe_id}{suffix}?draftId={}&expectedDraftRevision={}",
                    draft.id, draft.revision
                )
            )
            .await
            .status(),
            StatusCode::NOT_FOUND,
            "{suffix}"
        );
    }
}

#[tokio::test]
async fn model_dataframe_ids_are_private_and_missing_or_foreign_ids_are_indistinguishable() {
    let app = TestApp::spawn().await;
    let owner = common::test_user("alice@example.com");
    let owner_cookie = app.login_cookie("alice@example.com").await;
    let foreign_cookie = app.login_cookie("bob@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&owner.id, "private", 1)
        .await
        .unwrap();
    let draft = import_table(
        &app,
        &owner,
        session.id,
        "private",
        None,
        b"id,name\n1,private\n",
    )
    .await;
    let version = publish(&app, &owner_cookie, session.id, draft, "publish-private").await;
    let version_dataframe = version["tables"][0]["id"].as_str().unwrap();
    let live_draft = import_table(
        &app,
        &owner,
        session.id,
        "private-live-draft",
        None,
        b"id,name\n2,live\n",
    )
    .await;
    let view = workspace(&app, &owner_cookie, session.id).await;
    let draft_dataframe = view["draft"]["tables"][0]["id"].as_str().unwrap();

    for (prefix, dataframe_id, query) in [
        ("model-dataframes", version_dataframe, "".to_string()),
        (
            "draft-dataframes",
            draft_dataframe,
            format!(
                "?draftId={}&expectedDraftRevision={}",
                live_draft.id, live_draft.revision
            ),
        ),
    ] {
        for suffix in ["", "/rows", "/download"] {
            for (cookie, id) in [
                (&foreign_cookie, dataframe_id),
                (&owner_cookie, "2147483647"),
            ] {
                let response =
                    request(&app, cookie, &format!("/api/{prefix}/{id}{suffix}{query}")).await;
                assert_eq!(response.status(), StatusCode::NOT_FOUND, "{prefix}{suffix}");
                assert_eq!(json_body(response).await["code"], "not_found");
            }
        }
    }
}
