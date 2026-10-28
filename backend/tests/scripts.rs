mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::TestApp;
use serde_json::Value;
use tower::ServiceExt;
use zjudss::adapters::MODEL_TABLE_OUTPUTS;
use zjudss::application::study_planning::{
    ReadyStudyPlan, ResolvedSolver, SolverPreference, StudyRequest,
};
use zjudss::application::{CreateScriptRequest, PublishDraftRequest, ScriptSource};
use zjudss::auth::CurrentUser;

async fn publish_canonical_model(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
) -> ReadyStudyPlan {
    let draft = common::publish_model_bytes(
        app,
        user,
        session_id,
        "script-canonical",
        "master.dss",
        b"New Circuit.Test\n",
        None,
        None,
    )
    .await;
    let published = app
        .state
        .version_service
        .publish(
            user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: "publish-script-canonical".into(),
                description: None,
            },
        )
        .await
        .unwrap();
    ReadyStudyPlan {
        model_version_id: published.version.version.id,
        model_content_hash: published.version.version.content_hash,
        engine_snapshot_id: app.state.engine_snapshot.id.clone(),
        input_aliases: MODEL_TABLE_OUTPUTS
            .into_iter()
            .map(str::to_string)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        model_input_order: MODEL_TABLE_OUTPUTS
            .into_iter()
            .map(str::to_string)
            .collect(),
        request: StudyRequest::StaticPowerFlow {
            solver_preference: SolverPreference::Radial,
            requested_results: vec![],
        },
        function: "ds_radial_pf".into(),
        arity: 21,
        results: vec!["bus_phase".into(), "summary".into()],
        solver: Some(ResolvedSolver::Radial),
    }
}

const DIRECT_POWER_FLOW_CALL: &str = r#"[bus_phase, summary] = ds_radial_pf(
    bus, line, linecode, load, transformer, voltage_source, motor,
    transformer_winding, transformer_impedance, current_source, generator, ibr,
    switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para
);"#;

#[tokio::test]
async fn scripts_are_immutable_and_lazy_readable_without_static_rss_validation() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "scripts", 1)
        .await
        .unwrap();
    let long = (0..30)
        .map(|index| format!("output_{index} = bus"))
        .collect::<Vec<_>>()
        .join("\n");
    let first = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "study.rss".into(),
                content: long.clone(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
                study_plan: None,
            },
        )
        .await
        .unwrap();
    let second = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "copy.rss".into(),
                content: long.clone(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
                study_plan: None,
            },
        )
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.content_hash, second.content_hash);
    assert_eq!(first.validation_status, "valid");
    assert!(first.preview_truncated);
    assert_eq!(first.preview.lines().count(), 20);
    assert!(first.can_run);
    assert_eq!(first.disabled_reason, None);

    let detail = app
        .state
        .script_service
        .detail(&user, first.id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(detail.content, long);
    let unchecked = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "unsafe.rss".into(),
                content: "x = read(\"/tmp/source\")".into(),
                source: ScriptSource::Agent,
                agent_bundle_version: Some("test".into()),
                agent_skill_refs: vec![serde_json::json!({"id":"rss-generation"})],
                study_plan: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(unchecked.validation_status, "valid");
    assert_eq!(
        app.state
            .script_service
            .detail(&user, unchecked.id.parse().unwrap())
            .await
            .unwrap()
            .content,
        "x = read(\"/tmp/source\")"
    );

    let messages = app
        .state
        .message_repo
        .list_by_session(&user.id, session.id)
        .await
        .unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.kind == "script")
            .count(),
        3
    );
    assert!(
        messages
            .iter()
            .filter(|message| message.kind == "script")
            .all(|message| {
                let payload: Value =
                    serde_json::from_str(message.payload_json.as_deref().unwrap_or("{}")).unwrap();
                !payload["script"]["preview"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("output_29")
            })
    );
}

#[tokio::test]
async fn generic_agent_and_manual_or_uploaded_study_scripts_remain_compatible() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "script proof compatibility", 1)
        .await
        .unwrap();
    let cases = [
        (
            "generic-agent.rss",
            "result = bus",
            ScriptSource::Agent,
            Some("test".to_string()),
        ),
        (
            "manual-study.rss",
            "voltage = ds_radial_pf(bus)",
            ScriptSource::Manual,
            None,
        ),
        (
            "uploaded-study.rss",
            "voltage = ds_radial_pf(bus)",
            ScriptSource::Upload,
            None,
        ),
    ];

    for (name, content, source, bundle) in cases {
        let summary = app
            .state
            .script_service
            .create(
                &user,
                session.id,
                CreateScriptRequest {
                    name: name.into(),
                    content: content.into(),
                    source,
                    agent_bundle_version: bundle,
                    agent_skill_refs: vec![],
                    study_plan: None,
                },
            )
            .await
            .unwrap();
        let detail = app
            .state
            .script_service
            .detail(&user, summary.id.parse().unwrap())
            .await
            .unwrap();
        assert_eq!(detail.content, content);
    }
}

#[tokio::test]
async fn planned_agent_script_validation_is_deferred_to_tensoreval() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "planned input overwrite", 1)
        .await
        .unwrap();
    let plan = publish_canonical_model(&app, &user, session.id).await;
    let content = DIRECT_POWER_FLOW_CALL.replacen("[bus_phase, summary]", "bus", 1);

    let summary = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "overwrite-bus.rss".into(),
                content,
                source: ScriptSource::Agent,
                agent_bundle_version: Some("test".into()),
                agent_skill_refs: vec![],
                study_plan: Some(plan),
            },
        )
        .await
        .unwrap();

    assert_eq!(summary.validation_status, "valid");
    assert_eq!(summary.diagnostics["validation"], "deferred_to_tensoreval");
    assert!(summary.can_run);
    assert_eq!(summary.disabled_reason, None);
    assert_eq!(
        app.state
            .script_service
            .list(&user, session.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        app.state
            .message_repo
            .list_by_session(&user.id, session.id)
            .await
            .unwrap()
            .iter()
            .any(|message| message.kind == "script")
    );
}

#[tokio::test]
async fn script_routes_accept_only_single_rss_and_have_no_edit_or_delete() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "upload script", 1)
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let boundary = "script-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"run.rss\"\r\nContent-Type: text/plain\r\n\r\nresult = bus\r\n--{boundary}--\r\n"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/scripts", session.id))
                .header("cookie", &cookie)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    let id = body["script"]["id"].as_str().unwrap();
    let detail = app
        .router
        .clone()
        .oneshot(
            Request::get(format!("/api/scripts/{id}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail: Value =
        serde_json::from_slice(&to_bytes(detail.into_body(), 1024 * 1024).await.unwrap()).unwrap();
    assert_eq!(detail["script"]["content"], "result = bus");
    for method in ["PATCH", "DELETE"] {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/scripts/{id}"))
            .header("cookie", &cookie)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.router.clone().oneshot(request).await.unwrap().status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    let foreign = app.login_cookie("bob@example.com").await;
    assert_eq!(
        app.router
            .clone()
            .oneshot(
                Request::get(format!("/api/scripts/{id}"))
                    .header("cookie", foreign)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn script_text_endpoint_defers_rss_syntax_to_tensoreval() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "paste script", 1)
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{}/scripts/text", session.id))
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"manual.rss","content":"this is not valid RSS {{{"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(body["script"]["sourceKind"], "manual");
    assert_eq!(body["script"]["validationStatus"], "valid");
    assert_eq!(body["script"]["name"], "manual.rss");
}

#[tokio::test]
async fn script_resources_disappear_only_with_the_whole_session() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "cleanup", 1)
        .await
        .unwrap();
    let script = app
        .state
        .script_service
        .create(
            &user,
            session.id,
            CreateScriptRequest {
                name: "cleanup.rss".into(),
                content: "result = 1".into(),
                source: ScriptSource::Upload,
                agent_bundle_version: None,
                agent_skill_refs: vec![],
                study_plan: None,
            },
        )
        .await
        .unwrap();
    let cookie = app.login_cookie("alice@example.com").await;
    let token = common::session_deletion_token(&app, session.id, &cookie).await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{}?impactToken={token}", session.id))
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        app.state
            .script_service
            .detail(&user, script.id.parse().unwrap())
            .await
            .is_err()
    );
}
