mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use std::io::Cursor;
use tower::ServiceExt;
use zjudss::application::{
    ActivateVersionRequest, CandidateDecision, CandidatePurpose, PrepareStudyRequest,
    PublishDraftRequest, SelectStudyResultsRequest, StudyConfigurationError,
    StudyConfigurationState, UpdateStudyStructRequest, UploadFile, UploadSourceKind,
};
use zjudss::storage::StageLimits;

use common::TestApp;

async fn session(app: &TestApp, title: &str) -> (zjudss::auth::CurrentUser, i32) {
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, title, 1)
        .await
        .unwrap();
    (user, session.id)
}

async fn import_case(app: &TestApp, user: &zjudss::auth::CurrentUser, session_id: i32, id: &str) {
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .unwrap();
    let batch = app
        .state
        .case_import_service
        .apply(
            user,
            session_id,
            example.id.clone(),
            app.state.engine_snapshot.id.clone(),
            id.into(),
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| CandidateDecision {
            candidate_id: candidate.id.clone(),
            purpose: candidate.purpose.unwrap(),
            selected: true,
            table_name: (candidate.purpose == Some(CandidatePurpose::ModelTable))
                .then(|| candidate.proposed_alias.clone().unwrap()),
            display_name: None,
            alias: None,
        })
        .collect();
    app.state
        .data_resource_service
        .confirm_upload(user, batch.id.parse().unwrap(), decisions, None, None, id)
        .await
        .unwrap();
    let workspace = app
        .state
        .version_service
        .workspace(user, session_id)
        .await
        .unwrap();
    let draft = workspace.draft.unwrap().draft;
    let published = app
        .state
        .version_service
        .publish(
            user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: format!("publish-{id}"),
                description: Some("study fixture".into()),
            },
        )
        .await
        .unwrap();
    app.state
        .version_service
        .activate(
            user,
            published.version.version.id,
            ActivateVersionRequest {
                discard_draft: false,
                expected_draft: None,
            },
        )
        .await
        .unwrap();
}

async fn add_second_para_resource(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
) {
    let example = app
        .state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.importable)
        .unwrap();
    let batch = app
        .state
        .case_import_service
        .apply(
            user,
            session_id,
            example.id.clone(),
            app.state.engine_snapshot.id.clone(),
            "second-para".into(),
        )
        .await
        .unwrap();
    let decisions = batch
        .candidates
        .iter()
        .map(|candidate| {
            let para = candidate.proposed_alias.as_deref() == Some("para");
            CandidateDecision {
                candidate_id: candidate.id.clone(),
                purpose: if para {
                    CandidatePurpose::DataResource
                } else {
                    CandidatePurpose::Skip
                },
                selected: para,
                table_name: None,
                display_name: para.then(|| "备用潮流参数".into()),
                alias: para.then(|| "para_alt".into()),
            }
        })
        .collect();
    app.state
        .data_resource_service
        .confirm_upload(
            user,
            batch.id.parse().unwrap(),
            decisions,
            None,
            None,
            "publish-second-para",
        )
        .await
        .unwrap();
}

async fn publish_data_resource(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
    client_id: &str,
    bytes: &[u8],
) -> String {
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(bytes), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            client_id.into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: client_id.into(),
                file_name: format!("{client_id}.csv"),
                relative_path: None,
                media_type: Some("text/csv".into()),
                staged,
            }],
        )
        .await
        .unwrap();
    app.state
        .data_resource_service
        .publish_candidates(
            user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: Some("映射源".into()),
                alias: Some("mapping_source".into()),
            }],
        )
        .await
        .unwrap()
        .resources[0]
        .id
        .clone()
}

#[tokio::test]
async fn explicit_facade_creates_catalog_driven_cards_without_a_model() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "study-cards").await;

    let search = app
        .state
        .study_configuration_service
        .search("潮流", Some("ds_newton_raphson_pf"));
    assert_eq!(search.enabled.len(), 1);
    assert_eq!(search.enabled[0].id, "static_power_flow_newton");
    assert!(!search.enabled[0].differences.is_empty());

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"使用牛顿法计算潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        configuration.status,
        StudyConfigurationState::CollectingInputs
    );
    assert_eq!(configuration.facade.function, "ds_newton_raphson_pf");
    assert_eq!(
        configuration
            .inputs
            .iter()
            .filter(|input| input.kind == "model")
            .count(),
        20
    );
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    assert_eq!(para.kind, "struct");
    assert!(para.struct_card.as_ref().unwrap().fields.len() >= 5);
    assert_eq!(configuration.results.selected, vec!["bus_voltage"]);

    let error = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, configuration.id, configuration.revision)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "configuration_incomplete");
    assert!(matches!(error, StudyConfigurationError::Invalid(_, _)));
}

#[tokio::test]
async fn imported_case_binds_exact_active_tables_and_independent_para_resource() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "ready-study").await;
    import_case(&app, &user, session_id, "ready-case").await;

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"求解牛顿法潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        configuration.status,
        StudyConfigurationState::ReadyToConfirm
    );
    let model = configuration
        .inputs
        .iter()
        .filter(|input| input.kind == "model")
        .collect::<Vec<_>>();
    assert_eq!(model.len(), 20);
    assert!(
        model
            .iter()
            .all(|input| input.binding.model_table_name.is_some())
    );
    assert!(model.iter().all(|input| input.alias != "para"));
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    assert_eq!(para.resource_options.len(), 1);
    assert!(para.binding.resource_id.is_some());

    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, configuration.id, configuration.revision)
        .await
        .unwrap();
    assert_eq!(prepared.locked.function, "ds_newton_raphson_pf");
    assert_eq!(prepared.inputs.model.unwrap().tables.len(), 20);
    assert_eq!(prepared.locked.structs[0].resource_diff, json!([]));
    assert!(prepared.replacement_actions.is_empty());
}

#[tokio::test]
async fn struct_edits_require_a_replacement_choice_and_revision_is_optimistic() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "edit-study").await;
    import_case(&app, &user, session_id, "edit-case").await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_radial".into(),
                intent: json!({"request":"计算潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let mut values = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap()
        .struct_card
        .as_ref()
        .unwrap()
        .values
        .clone();
    let previous = values["line_model"].as_str().unwrap_or("modified");
    values["line_model"] = json!(if previous.eq_ignore_ascii_case("exact") {
        "modified"
    } else {
        "exact"
    });
    let edited = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values: values.clone(),
                replacement_action: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.status, StudyConfigurationState::CollectingInputs);
    let stale = app
        .state
        .study_configuration_service
        .select_results(
            &user,
            configuration.id,
            SelectStudyResultsRequest {
                expected_revision: configuration.revision,
                results: vec!["bus_voltage".into()],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(stale.code(), "configuration_stale");
    let resolved = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: edited.revision,
                values,
                replacement_action: Some(zjudss::application::ReplacementAction::Replace),
            },
        )
        .await
        .unwrap();
    assert_eq!(resolved.status, StudyConfigurationState::ReadyToConfirm);
    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, resolved.id, resolved.revision)
        .await
        .unwrap();
    assert_eq!(prepared.replacement_actions.len(), 1);
    assert_eq!(prepared.replacement_actions[0].action, "replace");
    assert!(!prepared.replacement_actions[0].differences.is_empty());
}

#[tokio::test]
async fn protection_configuration_uses_only_catalog_declared_model_inputs() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "protection").await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "protection_coordination".into(),
                intent: json!({"request":"保护配合"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let model_aliases = configuration
        .inputs
        .iter()
        .filter(|input| input.kind == "model")
        .map(|input| input.alias.as_str())
        .collect::<Vec<_>>();
    assert_eq!(model_aliases, vec!["fuse", "recloser", "relay"]);
}

#[tokio::test]
async fn multiple_compatible_struct_resources_are_never_guessed() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "ambiguous-para").await;
    import_case(&app, &user, session_id, "base-case").await;
    add_second_para_resource(&app, &user, session_id).await;

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_radial".into(),
                intent: json!({"request":"计算潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let para = configuration
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap();
    assert_eq!(para.resource_options.len(), 2);
    assert!(para.binding.resource_id.is_none());
    assert_eq!(
        configuration.status,
        StudyConfigurationState::CollectingInputs
    );
}

#[tokio::test]
async fn configuration_routes_are_user_scoped_revisioned_and_require_confirm_contract() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "api-config").await;
    let owner = app.login_cookie("alice@example.com").await;
    let foreign = app.login_cookie("bob@example.com").await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"牛顿潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let path = format!("/api/study-configurations/{}", configuration.id);
    let response = app
        .router
        .clone()
        .oneshot(
            Request::get(&path)
                .header("cookie", &foreign)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::put(format!("{path}/results"))
                .header("cookie", &owner)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"expectedRevision":configuration.revision,"results":["bus_voltage","summary"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let stale = app
        .router
        .clone()
        .oneshot(
            Request::put(format!("{path}/results"))
                .header("cookie", &owner)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"expectedRevision":configuration.revision,"results":["bus_voltage"]})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);

    let confirm = app
        .router
        .clone()
        .oneshot(
            Request::post(format!("{path}/confirm"))
                .header("cookie", &owner)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(confirm.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn confirmed_column_mapping_publishes_an_immutable_compatible_resource() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "mapping").await;
    let source = publish_data_resource(
        &app,
        &user,
        session_id,
        "mapping-source",
        b"profile,seconds,multiplier\ndaily,0,1.0\ndaily,3600,0.8\n",
    )
    .await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "qsts_radial".into(),
                intent: json!({"request":"时序潮流"}),
                derived_from_workflow_id: None,
            },
        )
        .await
        .unwrap();
    let mapped = app
        .state
        .study_configuration_service
        .map_resource(
            &user,
            configuration.id,
            "load_shape",
            zjudss::application::MapStudyResourceRequest {
                expected_revision: configuration.revision,
                source_resource_id: source.clone(),
                display_name: "日负荷曲线".into(),
                alias: "daily_shape".into(),
                mappings: vec![
                    zjudss::application::ColumnMapping {
                        source: "profile".into(),
                        target: "name".into(),
                    },
                    zjudss::application::ColumnMapping {
                        source: "seconds".into(),
                        target: "time_s".into(),
                    },
                    zjudss::application::ColumnMapping {
                        source: "multiplier".into(),
                        target: "p_multiplier".into(),
                    },
                ],
            },
        )
        .await
        .unwrap();
    let binding = mapped
        .inputs
        .iter()
        .find(|input| input.input_id == "load_shape")
        .unwrap()
        .binding
        .resource_id
        .as_ref()
        .unwrap()
        .clone();
    assert_ne!(binding, source);
    let resource = app
        .state
        .data_resource_service
        .get(&user, &binding)
        .await
        .unwrap();
    assert_eq!(resource.source_kind, "mapping");
    assert_eq!(
        resource.declared_schema_ref.as_deref(),
        Some("frames.load_shape")
    );
    assert_eq!(resource.lineage["sourceResourceId"], source);
}
