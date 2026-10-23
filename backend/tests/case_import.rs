mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use diesel::prelude::*;
use serde_json::{Value, json};
use tower::ServiceExt;

use common::TestApp;

#[tokio::test]
async fn applying_a_case_uses_model_import_and_changes_only_the_mutable_draft() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
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
        .find(|example| backend::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let response = app.router.clone().oneshot(
        Request::post(format!("/api/sessions/{}/draft/example", session.id))
            .header("cookie", &cookie).header("content-type", "application/json")
            .body(Body::from(json!({
                "exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,
                "clientImportId":"apply-case-1"
            }).to_string())).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let accepted: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(accepted["modelImport"]["sourceKind"], "case");
    let import_id = accepted["modelImport"]["id"].as_str().unwrap();

    let ready = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let response = app
                .router
                .clone()
                .oneshot(
                    Request::get(format!("/api/model-imports/{import_id}"))
                        .header("cookie", &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                    .unwrap();
            if matches!(
                value["modelImport"]["status"].as_str(),
                Some("ready_to_apply" | "awaiting_resolution" | "failed")
            ) {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(ready["modelImport"]["status"], "ready_to_apply", "{ready}");
    let candidate_ids = ready["modelImport"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(candidate_ids.len(), 21);
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/model-imports/{import_id}/resolve"))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"candidateIds":candidate_ids}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let workspace = app
        .state
        .version_service
        .workspace(&user, session.id)
        .await
        .unwrap();
    assert!(workspace.active.is_none());
    assert_eq!(workspace.draft.unwrap().dataframes.len(), 21);
    assert!(
        app.state
            .version_service
            .versions(&user, session.id)
            .await
            .unwrap()
            .is_empty()
    );
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type=diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut conn = app.pool.get().unwrap();
    let count = diesel::sql_query("SELECT count(*) count FROM simulation_runs WHERE session_id=?1")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .get_result::<Count>(&mut conn)
        .unwrap()
        .count;
    assert_eq!(count, 0);
}

#[tokio::test]
async fn case_catalog_hides_non_modelizable_entries_and_apply_is_user_scoped() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "case scope", 1)
        .await
        .unwrap();
    let alice = app.login_cookie("alice@example.com").await;
    let bob = app.login_cookie("bob@example.com").await;
    let list = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/examples")
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let list: Value =
        serde_json::from_slice(&to_bytes(list.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert!(
        list["examples"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["importable"] == true)
    );
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| backend::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/draft/example", session.id)).header("cookie", bob).header("content-type", "application/json").body(Body::from(json!({"exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,"clientImportId":"foreign"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
