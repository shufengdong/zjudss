mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use diesel::prelude::*;
use serde_json::json;
use std::io::Cursor;
use tower::ServiceExt;
use zjudss::application::{
    CandidateDecision, CandidatePurpose, PrepareStudyRequest, SelectStudyResultsRequest,
    StudyConfigurationError, StudyConfigurationState, UpdateStudyStructRequest, UploadFile,
    UploadSourceKind,
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
    assert!(workspace.draft.is_none());
    assert!(workspace.active.is_some());
}

async fn add_second_para_resource(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
) {
    let resources = zjudss::db::repositories::DataResourceRepo::new(app.pool.clone())
        .list_current_for_user(&user.id)
        .await
        .unwrap();
    let original = resources
        .iter()
        .find(|resource| resource.session_id == session_id && resource.alias == "para")
        .unwrap();
    let original = app
        .state
        .data_resource_service
        .get(user, &original.id)
        .await
        .unwrap();
    let preview=app.state.data_resource_service.preview_structured_replacement(user,&original.id,&original.content_hash,"frames.para",
        json!({"base_frequency_hz":50.0,"max_iter":30,"max_control_iter":10,"tol":1e-6,"line_model":"exact","result":"bus_phase,summary"})).await.unwrap();
    let resource = app
        .state
        .data_resource_service
        .confirm_structured_replacement(
            user,
            &preview.preview_token,
            zjudss::application::ReplacementAction::SaveAsNew,
        )
        .await
        .unwrap()
        .unwrap();
    assert_ne!(resource.id, original.id);
}

async fn add_result_selector_only_para_resource(
    app: &TestApp,
    user: &zjudss::auth::CurrentUser,
    session_id: i32,
) -> (String, String) {
    let original = app
        .state
        .data_resource_service
        .list(user, session_id, None, 100)
        .await
        .unwrap()
        .resources
        .into_iter()
        .find(|resource| resource.alias == "para")
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(user, &original.id, Some(2), None, Vec::new())
        .await
        .unwrap();
    let mut values = page
        .columns
        .into_iter()
        .zip(page.rows[0].clone())
        .collect::<serde_json::Map<_, _>>();
    values.insert("result".into(), json!("bus_voltage,load_state"));
    let preview = app
        .state
        .data_resource_service
        .preview_structured_replacement(
            user,
            &original.id,
            &original.content_hash,
            "frames.para",
            serde_json::Value::Object(values),
        )
        .await
        .unwrap();
    let child = app
        .state
        .data_resource_service
        .confirm_structured_replacement(
            user,
            &preview.preview_token,
            zjudss::application::ReplacementAction::SaveAsNew,
        )
        .await
        .unwrap()
        .unwrap();
    (original.id, child.id)
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
    assert_eq!(prepared.struct_overlays.len(), 1);
}

#[tokio::test]
async fn selector_values_are_server_owned_while_user_overrides_remain_private() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "server-owned-result-selector").await;
    import_case(&app, &user, session_id, "server-owned-result-selector-case").await;

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_radial".into(),
                intent: json!({"request":"计算径向潮流"}),
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
    let card = para.struct_card.as_ref().unwrap();
    assert_eq!(card.result_selector_fields, vec!["result"]);
    let baseline_result = card.values.get("result").cloned();
    let mut forged = card.values.clone();
    forged["result"] = json!("forged_result");
    let next_max_iter = forged["max_iter"].as_i64().unwrap() + 1;
    forged["max_iter"] = json!(next_max_iter);

    let updated = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values: forged,
            },
        )
        .await
        .unwrap();
    let updated_result = updated
        .inputs
        .iter()
        .find(|input| input.input_id == "para")
        .unwrap()
        .struct_card
        .as_ref()
        .unwrap()
        .values
        .get("result")
        .cloned();
    assert_eq!(updated_result, baseline_result);

    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, updated.id, updated.revision)
        .await
        .unwrap();
    assert_eq!(prepared.struct_overlays.len(), 1);
    assert_eq!(prepared.struct_overlays[0].input_id, "para");
    assert_eq!(
        prepared.struct_overlays[0].user_overrides,
        json!({"max_iter":next_max_iter})
    );
    assert_eq!(
        prepared.struct_overlays[0].selector_overrides,
        json!({"result":"bus_voltage"})
    );
    assert_eq!(
        prepared.struct_overlays[0].overrides,
        json!({"max_iter":next_max_iter,"result":"bus_voltage"})
    );
    assert_eq!(
        prepared.struct_overlays[0].effective_values["result"],
        json!("bus_voltage")
    );
}

#[tokio::test]
async fn result_selector_only_resource_does_not_create_ambiguity() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "selector-only-resource").await;
    import_case(&app, &user, session_id, "selector-only-resource-case").await;
    let (source_id, selector_only_id) =
        add_result_selector_only_para_resource(&app, &user, session_id).await;

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_radial".into(),
                intent: json!({"request":"计算径向潮流"}),
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
    assert_eq!(para.resource_options.len(), 1);
    assert_eq!(para.resource_options[0].resource_id, source_id);
    assert_ne!(para.resource_options[0].resource_id, selector_only_id);
    assert_eq!(
        para.binding.resource_id.as_deref(),
        Some(source_id.as_str())
    );
}

#[tokio::test]
async fn result_selector_only_resource_with_non_live_source_remains_a_candidate() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "selector-only-orphan").await;
    import_case(&app, &user, session_id, "selector-only-orphan-case").await;
    let (source_id, selector_only_id) =
        add_result_selector_only_para_resource(&app, &user, session_id).await;
    diesel::update(
        zjudss::db::schema::data_resources::table
            .filter(zjudss::db::schema::data_resources::id.eq(&source_id)),
    )
    .set(zjudss::db::schema::data_resources::status.eq("replaced"))
    .execute(&mut app.pool.get().unwrap())
    .unwrap();

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_radial".into(),
                intent: json!({"request":"计算径向潮流"}),
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
    assert_eq!(para.resource_options.len(), 1);
    assert_eq!(para.resource_options[0].resource_id, selector_only_id);
    assert_eq!(
        para.binding.resource_id.as_deref(),
        Some(selector_only_id.as_str())
    );
}

#[tokio::test]
async fn facade_without_result_selector_keeps_struct_fields_user_managed() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "fixed-results-struct").await;
    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "state_estimation_wls".into(),
                intent: json!({"request":"进行 WLS 状态估计"}),
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
    assert!(
        para.struct_card
            .as_ref()
            .unwrap()
            .result_selector_fields
            .is_empty()
    );
}

#[tokio::test]
async fn struct_edits_are_run_private_without_replacement_choice() {
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
                facade_id: "static_power_flow_newton".into(),
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
    let next_max_iter = values["max_iter"].as_i64().unwrap() + 7;
    let next_tol = 1e-7;
    values["max_iter"] = json!(next_max_iter);
    values["tol"] = json!(next_tol);
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
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.status, StudyConfigurationState::ReadyToConfirm);
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
    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, edited.id, edited.revision)
        .await
        .unwrap();
    assert_eq!(prepared.struct_overlays.len(), 1);
    assert_eq!(
        prepared.struct_overlays[0].user_overrides,
        json!({"max_iter":next_max_iter,"tol":next_tol})
    );
}

#[tokio::test]
async fn struct_without_resource_prepares_private_values_from_defaults() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "source-less-struct").await;
    import_case(&app, &user, session_id, "source-less-struct-case").await;
    let source = app
        .state
        .data_resource_service
        .list(&user, session_id, None, 100)
        .await
        .unwrap()
        .resources
        .into_iter()
        .find(|resource| resource.alias == "para")
        .unwrap();
    let page = app
        .state
        .data_resource_service
        .rows(&user, &source.id, Some(1), None, Vec::new())
        .await
        .unwrap();
    let source_values = serde_json::Value::Object(
        page.columns
            .into_iter()
            .zip(page.rows.into_iter().next().unwrap())
            .collect(),
    );
    diesel::update(
        zjudss::db::schema::data_resources::table
            .filter(zjudss::db::schema::data_resources::session_id.eq(session_id))
            .filter(zjudss::db::schema::data_resources::normalized_alias.eq("para")),
    )
    .set(zjudss::db::schema::data_resources::status.eq("replaced"))
    .execute(&mut app.pool.get().unwrap())
    .unwrap();

    let configuration = app
        .state
        .study_configuration_service
        .prepare(
            &user,
            session_id,
            PrepareStudyRequest {
                facade_id: "static_power_flow_newton".into(),
                intent: json!({"request":"使用默认参数计算径向潮流"}),
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
    assert!(para.binding.resource_id.is_none());
    assert!(para.resource_options.is_empty());
    assert_eq!(
        configuration.status,
        StudyConfigurationState::CollectingInputs
    );
    let completed = app
        .state
        .study_configuration_service
        .update_struct(
            &user,
            configuration.id,
            "para",
            UpdateStudyStructRequest {
                expected_revision: configuration.revision,
                values: source_values,
            },
        )
        .await
        .unwrap();
    assert_eq!(completed.status, StudyConfigurationState::ReadyToConfirm);

    let prepared = app
        .state
        .study_configuration_service
        .prepare_confirmation(&user, completed.id, completed.revision)
        .await
        .unwrap();
    assert_eq!(prepared.struct_overlays.len(), 1);
    assert!(prepared.struct_overlays[0].source_resource_id.is_none());
    assert!(
        !prepared.struct_overlays[0]
            .effective_values
            .as_object()
            .unwrap()
            .is_empty()
    );
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
async fn invalid_unique_struct_resource_is_left_for_parameter_card_completion() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "incomplete-para").await;
    import_case(&app, &user, session_id, "incomplete-para-case").await;

    let imported_para = app
        .state
        .data_resource_service
        .list(&user, session_id, None, 100)
        .await
        .unwrap()
        .resources
        .into_iter()
        .find(|resource| resource.declared_schema_ref.as_deref() == Some("frames.para"))
        .unwrap();
    let deletion = app
        .state
        .data_resource_service
        .preview_deletion(&user, &imported_para.id)
        .await
        .unwrap();
    app.state
        .data_resource_service
        .delete(&user, &imported_para.id, &deletion.impact_token)
        .await
        .unwrap();

    let incomplete_para = publish_data_resource(
        &app,
        &user,
        session_id,
        "incomplete-para-resource",
        b"max_iter,max_control_iter,tol,line_model\n100,30,0.000001,exact\n",
    )
    .await;
    diesel::update(
        zjudss::db::schema::data_resources::table
            .filter(zjudss::db::schema::data_resources::id.eq(&incomplete_para)),
    )
    .set(zjudss::db::schema::data_resources::declared_schema_ref.eq(Some("frames.para")))
    .execute(&mut app.pool.get().unwrap())
    .unwrap();

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
    assert_eq!(para.resource_options.len(), 1);
    assert!(para.binding.resource_id.is_none());
    assert_eq!(
        configuration.status,
        StudyConfigurationState::CollectingInputs
    );
}

#[tokio::test]
async fn unique_struct_resource_read_failures_are_not_silently_ignored() {
    let app = TestApp::spawn().await;
    let (user, session_id) = session(&app, "unreadable-para").await;
    let para = publish_data_resource(
        &app,
        &user,
        session_id,
        "unreadable-para-resource",
        b"base_frequency_hz,max_iter,max_control_iter,tol,line_model\n50,100,30,0.000001,exact\n",
    )
    .await;
    diesel::update(
        zjudss::db::schema::data_resources::table
            .filter(zjudss::db::schema::data_resources::id.eq(&para)),
    )
    .set(zjudss::db::schema::data_resources::declared_schema_ref.eq(Some("frames.para")))
    .execute(&mut app.pool.get().unwrap())
    .unwrap();
    let artifact_id = zjudss::db::schema::data_resources::table
        .filter(zjudss::db::schema::data_resources::id.eq(&para))
        .select(zjudss::db::schema::data_resources::normalized_artifact_id)
        .first::<Option<i32>>(&mut app.pool.get().unwrap())
        .unwrap()
        .unwrap();
    diesel::update(
        zjudss::db::schema::artifacts::table
            .filter(zjudss::db::schema::artifacts::id.eq(artifact_id)),
    )
    .set(zjudss::db::schema::artifacts::storage_path.eq("missing/unreadable-para.parquet"))
    .execute(&mut app.pool.get().unwrap())
    .unwrap();

    let error = app
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
        .unwrap_err();
    assert_eq!(error.code(), "dataframe_read_failed");
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
