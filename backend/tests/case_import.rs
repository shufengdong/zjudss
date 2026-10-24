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
    let manifest_source = example
        .files
        .iter()
        .find(|file| file.role == "manifest")
        .unwrap();
    let bundle_root = app
        .state
        .engine_snapshot
        .engine_root
        .join(&manifest_source.source_path)
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::write(bundle_root.join("undeclared.txt"), b"must not be imported").unwrap();
    let expected_summary = json!({
        "exampleId": example.id,
        "title": example.title,
        "categoryPath": example.category_path,
        "bundlePath": example.bundle_path,
        "engineResourceSnapshot": app.state.engine_snapshot.id,
        "manifestSchemaVersion": example.manifest.as_ref().unwrap().schema_version,
        "sourceTreeHash": example.manifest.as_ref().unwrap().source.tree_sha256,
        "caseTreeHash": example.tree_hash,
        "fileCount": example.files.len(),
    });
    std::fs::remove_dir_all(&app.live_engine_root).unwrap();
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
    let accepted_files = accepted["modelImport"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["displayName"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        accepted_files,
        vec!["manifest.json", "dss/model.dss", "main.rss"]
    );
    let import_id = accepted["modelImport"]["id"].as_str().unwrap();

    #[derive(QueryableByName)]
    struct SourceSummary {
        #[diesel(sql_type = diesel::sql_types::Text)]
        source_summary_json: String,
    }
    let mut conn = app.pool.get().unwrap();
    let stored = diesel::sql_query("SELECT source_summary_json FROM model_imports WHERE id=?1")
        .bind::<diesel::sql_types::Integer, _>(import_id.parse::<i32>().unwrap())
        .get_result::<SourceSummary>(&mut conn)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored.source_summary_json).unwrap(),
        expected_summary
    );
    drop(conn);

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
async fn applying_a_case_rejects_post_publication_mutation_atomically() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "mutated case", 1)
        .await
        .unwrap();
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| backend::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let changed = example
        .files
        .iter()
        .find(|file| file.role == "main-rss")
        .unwrap();
    std::fs::write(
        app.state
            .engine_snapshot
            .engine_root
            .join(&changed.source_path),
        b"changed after publication\n",
    )
    .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/draft/example", session.id))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "exampleId": example.id,
                        "engineResourceSnapshot": app.state.engine_snapshot.id,
                        "clientImportId": "mutated-case"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let error: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(error["code"], "case_not_modelizable");

    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut conn = app.pool.get().unwrap();
    let count = diesel::sql_query(
        "SELECT count(*) count FROM model_imports WHERE client_import_id='mutated-case'",
    )
    .get_result::<Count>(&mut conn)
    .unwrap()
    .count;
    assert_eq!(count, 0);
    assert_eq!(
        std::fs::read_dir(app.state.data_dir.join("staging"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn case_catalog_preserves_directories_and_includes_invalid_leaves() {
    let app = TestApp::spawn().await;
    let alice = app.login_cookie("alice@example.com").await;
    let response = app
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
    assert_eq!(response.status(), StatusCode::OK);
    let catalog: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        catalog["engineResourceSnapshot"],
        app.state.engine_snapshot.id
    );
    assert!(catalog.get("examples").is_none());
    let tree = catalog["tree"].as_array().unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0]["type"], "directory");
    assert_eq!(tree[0]["name"], "E2E");
    assert_eq!(tree[0]["path"], json!(["E2E"]));
    let demo = &tree[0]["children"][0];
    assert_eq!(demo["type"], "directory");
    assert_eq!(demo["name"], "Demo");
    assert_eq!(demo["path"], json!(["E2E", "Demo"]));
    let leaves = demo["children"].as_array().unwrap();
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0]["example"]["title"], "Broken");
    assert_eq!(leaves[0]["example"]["importable"], false);
    assert_eq!(leaves[1]["example"]["title"], "E2E demo");
    assert_eq!(leaves[1]["example"]["importable"], true);
    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(!serialized.contains(app.live_engine_root.to_str().unwrap()));
    assert!(!serialized.contains("sourcePath"));
}

#[tokio::test]
async fn case_detail_exposes_normalized_manifest_without_source_paths() {
    let app = TestApp::spawn().await;
    let alice = app.login_cookie("alice@example.com").await;
    let valid = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .unwrap();
    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/examples/{}", valid.id))
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(detail["categoryPath"], json!(["E2E", "Demo"]));
    assert_eq!(detail["bundlePath"], "E2E/Demo/E2E demo.rssbundle");
    assert_eq!(detail["entryPath"], "main.rss");
    assert_eq!(detail["manifest"]["schemaVersion"], 1);
    assert_eq!(detail["manifest"]["artifact"], "mems.dss-rss-bundle");
    assert_eq!(detail["manifest"]["study"]["kind"], "execution_plan");
    assert_eq!(
        detail["manifest"]["study"]["compatibility"],
        "opendss-native"
    );
    assert_eq!(detail["manifest"]["study"]["results"], json!(["voltage"]));
    assert_eq!(detail["manifest"]["source"]["entry"], "dss/model.dss");
    assert_eq!(detail["manifest"]["source"]["root"], "dss");
    assert!(detail["manifest"]["source"]["treeSha256"].is_string());
    assert_eq!(detail["manifest"]["inputs"], json!([]));
    let roles = detail["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["role"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roles, vec!["manifest", "dss-entry", "main-rss"]);
    let serialized = serde_json::to_string(&detail).unwrap();
    assert!(!serialized.contains(app.live_engine_root.to_str().unwrap()));
    assert!(!serialized.contains("sourcePath"));

    let invalid = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| !example.importable)
        .unwrap();
    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/examples/{}", invalid.id))
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let detail: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert!(detail["manifest"].is_null());
    assert!(detail["entryPath"].is_null());
    assert_eq!(detail["diagnostics"], json!(["manifest_invalid"]));
}

#[tokio::test]
async fn case_catalog_searches_manifest_metadata_and_keeps_ancestors() {
    let app = TestApp::spawn().await;
    let alice = app.login_cookie("alice@example.com").await;
    for query in ["voltage", "model.dss", "execution_plan"] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::get(format!("/api/examples?query={query}"))
                    .header("cookie", &alice)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let catalog: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(catalog["tree"][0]["name"], "E2E");
        assert_eq!(catalog["tree"][0]["children"][0]["name"], "Demo");
        let leaves = catalog["tree"][0]["children"][0]["children"]
            .as_array()
            .unwrap();
        assert_eq!(leaves.len(), 1, "query={query}: {catalog}");
        assert_eq!(leaves[0]["example"]["title"], "E2E demo");
    }
    let response = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/examples?studyType=execution_plan")
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let catalog: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(
        catalog["tree"][0]["children"][0]["children"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let response = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/examples?query=no-such-case")
                .header("cookie", &alice)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let catalog: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(catalog["tree"], json!([]));
}

#[tokio::test]
async fn case_apply_is_user_scoped() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "case scope", 1)
        .await
        .unwrap();
    let bob = app.login_cookie("bob@example.com").await;
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
