mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use common::TestApp;
use zjudss::application::ReplacementAction;

#[tokio::test]
async fn applying_a_case_creates_one_recoverable_upload_batch_without_publishing() {
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
        .find(|example| zjudss::application::CaseImportService::is_modelizable(example))
        .unwrap();
    std::fs::remove_dir_all(&app.live_engine_root).unwrap();
    let response=app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/draft/example",session.id))
        .header("cookie",&cookie).header("content-type","application/json")
        .body(Body::from(json!({"exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,"clientBatchId":"apply-case-1"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let batch = &body["uploadBatch"];
    assert_eq!(batch["sourceKind"], "case");
    assert_eq!(batch["status"], "ready_to_confirm");
    let materialized_bundle = app
        .data_dir
        .join("upload-work")
        .join(batch["id"].as_str().unwrap())
        .join("input.rssbundle");
    assert!(materialized_bundle.join("manifest.json").is_file());
    assert!(materialized_bundle.join("dss/model.dss").is_file());
    let candidates = batch["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 21);
    assert_eq!(
        candidates
            .iter()
            .filter(|value| value["purpose"] == "model_table")
            .count(),
        20
    );
    let para = candidates
        .iter()
        .find(|value| value["proposedAlias"] == "para")
        .unwrap();
    assert_eq!(para["purpose"], "data_resource");
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate["proposedAlias"] != "projection_diagnostic")
    );
    assert_eq!(
        batch["diagnostics"]["caseProjection"]["diagnosticOutputs"][0]["name"],
        "projection_diagnostic"
    );
    assert_eq!(
        batch["diagnostics"]["caseProjection"]["diagnosticOutputs"][0]["rows"][0]["id"],
        "stub-21"
    );
    assert!(
        app.state
            .version_service
            .versions(&user, session.id)
            .await
            .unwrap()
            .is_empty()
    );
}

async fn rejected_case_source(mutate: bool) {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "bad case", 1)
        .await
        .unwrap();
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| zjudss::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let source = example
        .files
        .iter()
        .find(|file| file.role == "main-rss")
        .unwrap();
    let path = app
        .state
        .engine_snapshot
        .engine_root
        .join(&source.source_path);
    if mutate {
        std::fs::write(path, b"changed after publication\n").unwrap();
    } else {
        std::fs::remove_file(path).unwrap();
    }
    let response=app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/draft/example",session.id))
        .header("cookie",&cookie).header("content-type","application/json")
        .body(Body::from(json!({"exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,"clientBatchId":"bad-case"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let error: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(error["code"], "case_not_modelizable");
    let count = app
        .state
        .upload_batch_service
        .count_by_client_id(&user, session.id, "bad-case")
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn applying_a_case_rejects_post_publication_mutation_atomically() {
    rejected_case_source(true).await;
}

#[tokio::test]
async fn applying_a_case_rejects_missing_published_file_atomically() {
    rejected_case_source(false).await;
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
    assert_eq!(detail["manifest"]["schemaVersion"], 2);
    assert_eq!(detail["manifest"]["artifact"], "mems.dss-rss-bundle");
    assert_eq!(detail["manifest"]["study"]["kind"], "static_power_flow");
    assert_eq!(
        detail["manifest"]["study"]["compatibility"],
        "opendss-native"
    );
    assert_eq!(detail["manifest"]["study"]["results"], json!(["voltage"]));
    assert_eq!(
        detail["manifest"]["projection"]["profile"],
        "dss_static_newton_v1"
    );
    assert_eq!(
        detail["manifest"]["projection"]["managedEntry"],
        "managed.rss"
    );
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
    assert_eq!(
        roles,
        vec!["manifest", "dss-entry", "main-rss", "managed-rss"]
    );
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
    for query in ["voltage", "model.dss", "static_power_flow"] {
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
            Request::get("/api/examples?studyType=static_power_flow")
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
        .find(|example| zjudss::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/draft/example", session.id)).header("cookie", bob).header("content-type", "application/json").body(Body::from(json!({"exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,"clientImportId":"foreign"}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn confirming_cases_atomically_activates_model_and_replaces_case_resources() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;
    let session = app
        .state
        .session_repo
        .create(&user.id, "switch", 1)
        .await
        .unwrap();
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| zjudss::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let mut previous_resource = String::new();
    let mut version = Value::Null;
    for index in 0..2 {
        if index == 1 {
            use diesel::prelude::*;
            let mut conn = app.pool.get().unwrap();
            diesel::sql_query("INSERT INTO workspace_drafts(workspace_id,base_version_id,revision,diagnostics_json,created_at_ms,updated_at_ms) SELECT id,active_version_id,1,'{}',1,1 FROM workspaces WHERE session_id=?1")
                .bind::<diesel::sql_types::Integer,_>(session.id).execute(&mut conn).unwrap();
            diesel::sql_query("INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) SELECT (SELECT d.id FROM workspace_drafts d JOIN workspaces w ON w.id=d.workspace_id WHERE w.session_id=?1),'previous_case_only',m.artifact_id,m.schema_json,m.row_count,m.content_hash,1 FROM model_dataframes m JOIN workspaces w ON w.active_version_id=m.version_id WHERE w.session_id=?1 LIMIT 1")
                .bind::<diesel::sql_types::Integer,_>(session.id).execute(&mut conn).unwrap();
            diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+1 WHERE storage_scope_id=?1 AND artifact_id IN(SELECT artifact_id FROM draft_dataframes)")
                .bind::<diesel::sql_types::Integer,_>(user.storage_scope_id).execute(&mut conn).unwrap();
        }
        let before = app
            .state
            .version_service
            .workspace(&user, session.id)
            .await
            .unwrap();
        let expected = before
            .draft
            .map(|draft| json!({"id":draft.draft.id,"revision":draft.draft.revision}));
        let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{}/draft/example",session.id))
            .header("cookie", &cookie).header("content-type", "application/json")
            .body(Body::from(json!({"exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,"clientBatchId":format!("switch-{index}")}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        let batch = &body["uploadBatch"];
        let response = app
            .router
            .clone()
            .oneshot(
                Request::post(format!(
                    "/api/upload-batches/{}/confirm",
                    batch["id"].as_str().unwrap()
                ))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"decisions":[],"expectedDraft":expected,"clientOperationId":format!("switch-confirm-{index}")})
                        .to_string(),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let confirmed: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
                .unwrap();
        let confirmed = &confirmed["uploadConfirmation"];
        assert_eq!(confirmed["status"], "confirmed", "{confirmed}");
        assert!(
            confirmed["modelReceipt"]["versionId"].is_number(),
            "{confirmed}"
        );
        if index == 0 {
            version = confirmed["modelReceipt"]["versionId"].clone();
        } else {
            assert_eq!(confirmed["modelReceipt"]["versionId"], version);
        }
        let workspace = app
            .state
            .version_service
            .workspace(&user, session.id)
            .await
            .unwrap();
        assert!(workspace.draft.is_none());
        assert_eq!(workspace.active.as_ref().unwrap().dataframes.len(), 20);
        assert!(
            workspace
                .active
                .as_ref()
                .unwrap()
                .dataframes
                .iter()
                .all(|frame| frame.table_name != "previous_case_only")
        );
        if index == 1 {
            let repo = zjudss::db::repositories::DataResourceRepo::new(app.pool.clone());
            let old = repo
                .find_record(&user.id, &previous_resource)
                .await
                .unwrap();
            assert_eq!(old.status, "replaced");
            assert!(old.normalized_artifact_id.is_some());
            assert!(old.replaced_by_resource_id.is_some());
        }
        let resource_id = confirmed["resources"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(resource_id, previous_resource);
        previous_resource = resource_id;
    }
    assert_eq!(
        app.state
            .version_service
            .versions(&user, session.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

async fn prepare_switch(app: &TestApp, cookie: &str, session_id: i32, client: &str) -> Value {
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| zjudss::application::CaseImportService::is_modelizable(example))
        .unwrap();
    let response = app.router.clone().oneshot(Request::post(format!("/api/sessions/{session_id}/draft/example"))
        .header("cookie",cookie).header("content-type","application/json")
        .body(Body::from(json!({"exampleId":example.id,"engineResourceSnapshot":app.state.engine_snapshot.id,"clientBatchId":client}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    body["uploadBatch"].clone()
}

async fn confirm_switch(app: &TestApp, cookie: &str, batch: &Value, operation: &str) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/upload-batches/{}/confirm",
                batch["id"].as_str().unwrap()
            ))
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"decisions":[],"clientOperationId":operation}).to_string(),
            ))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    body["uploadConfirmation"].clone()
}

#[tokio::test]
async fn case_confirmation_preserves_legacy_replacement_action_compatibility() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let cookie = app.login_cookie("alice@example.com").await;

    let cancelled_session = app
        .state
        .session_repo
        .create(&user.id, "cancel case", 1)
        .await
        .unwrap();
    let cancelled = prepare_switch(&app, &cookie, cancelled_session.id, "cancel-case").await;
    let cancelled_confirmation = app
        .state
        .data_resource_service
        .confirm_upload(
            &user,
            cancelled["id"].as_str().unwrap().parse().unwrap(),
            vec![],
            Some(ReplacementAction::Cancel),
            None,
            "cancel-case",
        )
        .await
        .unwrap();
    assert_eq!(cancelled_confirmation.status, "discarded");

    for (index, action) in [ReplacementAction::Replace, ReplacementAction::SaveAsNew]
        .into_iter()
        .enumerate()
    {
        let session = app
            .state
            .session_repo
            .create(&user.id, &format!("case action {index}"), 1)
            .await
            .unwrap();
        let batch =
            prepare_switch(&app, &cookie, session.id, &format!("case-action-{index}")).await;
        let batch_id = batch["id"].as_str().unwrap().parse().unwrap();
        let operation = format!("case-action-confirm-{index}");
        let confirmation = app
            .state
            .data_resource_service
            .confirm_upload(&user, batch_id, vec![], Some(action), None, &operation)
            .await
            .unwrap();
        assert_eq!(confirmation.status, "confirmed");
        assert_eq!(confirmation.model_status, "published");
        assert_eq!(confirmation.resource_status, "published");
        assert!(!confirmation.resources.is_empty());

        if action == ReplacementAction::Replace {
            let conflict = app
                .state
                .data_resource_service
                .confirm_upload(
                    &user,
                    batch_id,
                    vec![],
                    Some(ReplacementAction::SaveAsNew),
                    None,
                    &operation,
                )
                .await
                .unwrap_err();
            assert_eq!(conflict.code(), "idempotency_conflict");
        }
    }
}

#[tokio::test]
async fn case_switch_failure_preserves_active_model_and_resources() {
    use diesel::prelude::*;
    use zjudss::db::schema;
    for failure in ["invalid_resource", "stale_preview", "user_conflict"] {
        let app = TestApp::spawn().await;
        let user = common::test_user("alice@example.com");
        let cookie = app.login_cookie("alice@example.com").await;
        let session = app
            .state
            .session_repo
            .create(&user.id, failure, 1)
            .await
            .unwrap();
        let first = prepare_switch(&app, &cookie, session.id, "first").await;
        let installed = confirm_switch(&app, &cookie, &first, "install").await;
        assert_eq!(installed["status"], "confirmed");
        let old_id = installed["resources"][0]["id"].as_str().unwrap();
        if failure == "user_conflict" {
            let repo = zjudss::db::repositories::DataResourceRepo::new(app.pool.clone());
            let mut manual = repo.find_record(&user.id, old_id).await.unwrap();
            let mut conn = app.pool.get().unwrap();
            diesel::update(schema::data_resources::table.find(old_id))
                .set((
                    schema::data_resources::alias.eq("old_para"),
                    schema::data_resources::normalized_alias.eq("old_para"),
                ))
                .execute(&mut conn)
                .unwrap();
            manual.id = "manual-user-data".into();
            manual.source_batch_id = None;
            manual.source_artifact_id = None;
            manual.lineage_json = "{}".into();
            diesel::insert_into(schema::data_resources::table)
                .values(&manual)
                .execute(&mut conn)
                .unwrap();
            diesel::sql_query("UPDATE storage_scope_artifacts SET ref_count=ref_count+1 WHERE storage_scope_id=?1 AND artifact_id=?2")
                .bind::<diesel::sql_types::Integer,_>(user.storage_scope_id).bind::<diesel::sql_types::Integer,_>(manual.normalized_artifact_id.unwrap()).execute(&mut conn).unwrap();
        }
        let second = prepare_switch(&app, &cookie, session.id, "second").await;
        {
            let mut conn = app.pool.get().unwrap();
            if failure == "invalid_resource" {
                diesel::update(
                    schema::upload_candidates::table
                        .filter(
                            schema::upload_candidates::batch_id.eq(second["id"]
                                .as_str()
                                .unwrap()
                                .parse::<i32>()
                                .unwrap()),
                        )
                        .filter(schema::upload_candidates::purpose.eq("data_resource")),
                )
                .set(schema::upload_candidates::row_count.eq(Option::<i64>::None))
                .execute(&mut conn)
                .unwrap();
            } else if failure == "stale_preview" {
                diesel::update(schema::data_resources::table.find(old_id))
                    .set((
                        schema::data_resources::alias.eq("changed_para"),
                        schema::data_resources::normalized_alias.eq("changed_para"),
                    ))
                    .execute(&mut conn)
                    .unwrap();
            }
        }
        let rejected = confirm_switch(&app, &cookie, &second, "reject").await;
        assert_eq!(
            rejected["status"], "ready_to_confirm",
            "{failure}: {rejected}"
        );
        assert_eq!(rejected["modelStatus"], "failed");
        assert_eq!(rejected["resourceStatus"], "failed");
        let workspace = app
            .state
            .version_service
            .workspace(&user, session.id)
            .await
            .unwrap();
        assert_eq!(
            workspace.active.unwrap().version.id,
            installed["modelReceipt"]["versionId"].as_i64().unwrap() as i32
        );
        assert!(workspace.draft.is_none());
        let repo = zjudss::db::repositories::DataResourceRepo::new(app.pool.clone());
        assert_eq!(
            repo.find_record(&user.id, old_id).await.unwrap().status,
            "live"
        );
        assert_eq!(
            app.state
                .version_service
                .versions(&user, session.id)
                .await
                .unwrap()
                .len(),
            1
        );
        let replay = confirm_switch(&app, &cookie, &second, "reject").await;
        assert_eq!(replay["diagnostics"], rejected["diagnostics"]);
    }
}
