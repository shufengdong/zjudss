mod common;

use std::collections::BTreeSet;
use std::io::Cursor;
use std::sync::Arc;

use diesel::RunQueryDsl;
use rig::tool::{Tool, ToolContext};
use tokio::sync::{Mutex, Semaphore};
use zjudss::adapters::StubEngine;
use zjudss::agent::knowledge::AgentKnowledge;
use zjudss::application::{
    AnalysisInput, AnalysisInputSource, AnalysisRequest, ArtifactMetadata, CandidateDecision,
    CandidatePurpose, EventWriter, PublishDraftRequest, UploadFile, UploadSourceKind,
};
use zjudss::auth::CurrentUser;
use zjudss::db::repositories::VersionSnapshot;
use zjudss::storage::StageLimits;
use zjudss::testing::{DatasetFixture, EngineScenario, ScenarioEngine};
use zjudss::tools::{self, ToolServices};

use common::TestApp;

fn principal(key: &str) -> CurrentUser {
    common::test_user(key)
}

fn services(app: &TestApp, principal: CurrentUser) -> Arc<ToolServices> {
    services_for_session(app, principal, 0)
}

fn services_for_session(
    app: &TestApp,
    principal: CurrentUser,
    session_id: i32,
) -> Arc<ToolServices> {
    Arc::new(ToolServices {
        engine: Arc::new(StubEngine),
        writer: EventWriter::new(
            principal.id.clone(),
            session_id,
            app.state.message_repo.clone(),
            app.state.events.clone(),
        ),
        session_id,
        request_message_id: 0,
        publication_authorized: false,
        semaphore: Arc::new(Semaphore::new(1)),
        scope: principal.id.clone(),
        principal,
        model_patch: app.state.model_patch_service.clone(),
        model_dataframes: app.state.model_dataframe_read_service.clone(),
        versions: app.state.version_service.clone(),
        runs: app.state.run_service.clone(),
        engine_snapshot: app.state.engine_snapshot.clone(),
        agent_knowledge: app.state.agent_knowledge.clone(),
        loaded_skills: Arc::new(Mutex::new(BTreeSet::new())),
        analysis_attempts: Arc::new(Mutex::new(0)),
        inspected_workspace: Arc::new(Mutex::new(false)),
        inspected_workspace_draft: Arc::new(Mutex::new(None)),
        study_configurations: app.state.study_configuration_service.clone(),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
        data_resources: app.state.data_resource_service.clone(),
    })
}

async fn publish_tool_resource(app: &TestApp, user: &CurrentUser, session_id: i32) -> String {
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"id,value\n1,2\n"), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            user,
            session_id,
            "diagnostic-resource".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "diagnostic-resource".into(),
                file_name: "diagnostic.csv".into(),
                relative_path: Some("diagnostic.csv".into()),
                media_type: Some("text/csv".into()),
                staged,
            }],
        )
        .await
        .unwrap();
    app.state
        .upload_confirmation_service
        .publish_candidates(
            user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: None,
                alias: Some("diagnostic_resource".into()),
            }],
        )
        .await
        .unwrap()
        .resources
        .remove(0)
        .id
}

async fn publish_active_version(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    client_id: &str,
    bytes: &[u8],
) -> VersionSnapshot {
    let draft = common::publish_model_bytes(
        app,
        user,
        session_id,
        client_id,
        "bus.csv",
        bytes,
        None,
        Some("bus"),
    )
    .await;
    app.state
        .version_service
        .publish(
            user,
            session_id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: format!("publish-{client_id}"),
                description: Some(format!("version {client_id}")),
            },
        )
        .await
        .unwrap()
        .version
}

fn insert_direct_dataset(
    app: &TestApp,
    session_id: i32,
    version: &VersionSnapshot,
    suffix: &str,
) -> i32 {
    let frame = version.dataframes.first().unwrap();
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("INSERT INTO script_resources(session_id,artifact_id,name,source_kind,content_hash,validation_status,required_aliases_json,outputs_json,diagnostics_json,agent_skill_refs_json,created_at_ms) VALUES(?1,?2,?3,'manual',?4,'valid','[]','[]','{}','[]',1)")
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .bind::<diesel::sql_types::Integer, _>(frame.artifact_id)
        .bind::<diesel::sql_types::Text, _>(format!("{suffix}.rss"))
        .bind::<diesel::sql_types::Text, _>(&frame.content_hash)
        .execute(&mut conn)
        .unwrap();
    let script_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result::<i32>(&mut conn)
    .unwrap();
    diesel::sql_query("INSERT INTO simulation_runs(session_id,model_version_id,model_content_hash,script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'succeeded','completed','test','[]',2)")
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .bind::<diesel::sql_types::Integer, _>(version.version.id)
        .bind::<diesel::sql_types::Text, _>(&version.version.content_hash)
        .bind::<diesel::sql_types::Integer, _>(script_id)
        .bind::<diesel::sql_types::Integer, _>(frame.artifact_id)
        .bind::<diesel::sql_types::Text, _>(&frame.content_hash)
        .bind::<diesel::sql_types::Text, _>(format!("run-{suffix}"))
        .execute(&mut conn)
        .unwrap();
    let run_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result::<i32>(&mut conn)
    .unwrap();
    diesel::sql_query("INSERT INTO result_datasets(run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) VALUES(?1,?2,?3,?4,?5,1,1,3)")
        .bind::<diesel::sql_types::Integer, _>(run_id)
        .bind::<diesel::sql_types::Integer, _>(frame.artifact_id)
        .bind::<diesel::sql_types::Text, _>(format!("result-{suffix}"))
        .bind::<diesel::sql_types::Text, _>(&frame.schema_json)
        .bind::<diesel::sql_types::Text, _>(&frame.content_hash)
        .execute(&mut conn)
        .unwrap();
    diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result(&mut conn)
    .unwrap()
}

fn insert_request_message(app: &TestApp, session_id: i32) -> i32 {
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("INSERT INTO messages(session_id,role,kind,content,created_at_ms) VALUES(?1,'assistant','tool_call','analysis',1)")
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .execute(&mut conn)
        .unwrap();
    diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result(&mut conn)
    .unwrap()
}

#[derive(diesel::QueryableByName)]
struct AnalysisInputSnapshotRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_kind: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    dataset_id: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    resource_id: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Integer>)]
    model_dataframe_id: Option<i32>,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_schema_json: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_schema_hash: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    source_content_hash: String,
}

#[tokio::test]
async fn data_resource_tools_list_read_rename_and_promote_owned_session_resources() {
    let app = TestApp::spawn().await;
    let user = principal("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "resource tools", 1)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(b"x,value\n1,2\n"), StageLimits::default())
        .unwrap();
    let batch = app
        .state
        .upload_batch_service
        .accept(
            &user,
            session.id,
            "tool-resource".into(),
            UploadSourceKind::Upload,
            vec![UploadFile {
                client_file_id: "tool-resource".into(),
                file_name: "tool-resource.csv".into(),
                relative_path: Some("tool-resource.csv".into()),
                media_type: Some("text/csv".into()),
                staged,
            }],
        )
        .await
        .unwrap();
    let published = app
        .state
        .upload_confirmation_service
        .publish_candidates(
            &user,
            batch.id.parse().unwrap(),
            vec![CandidateDecision {
                candidate_id: batch.candidates[0].id.clone(),
                purpose: CandidatePurpose::DataResource,
                selected: true,
                table_name: None,
                display_name: Some("工具数据".into()),
                alias: Some("tool_data".into()),
            }],
        )
        .await
        .unwrap();
    let resource_id = published.resources[0].id.clone();
    let tool_services = services_for_session(&app, user.clone(), session.id);
    let mut context = ToolContext::new();

    let listed = tools::list_data_resources::ListDataResourcesTool::new(tool_services.clone())
        .call(
            &mut context,
            tools::list_data_resources::Args {
                cursor: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();
    assert_eq!(listed["resources"][0]["id"], resource_id);
    let read = tools::read_data_resource::ReadDataResourceTool::new(tool_services.clone())
        .call(
            &mut context,
            tools::read_data_resource::Args {
                resource_id: resource_id.clone(),
                columns: vec!["value".into()],
                limit: Some(10),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(read["page"]["rows"][0][0], "2");
    let renamed =
        tools::update_data_resource_alias::UpdateDataResourceAliasTool::new(tool_services.clone())
            .call(
                &mut context,
                tools::update_data_resource_alias::Args {
                    resource_id: resource_id.clone(),
                    alias: "renamed_tool_data".into(),
                },
            )
            .await
            .unwrap();
    assert_eq!(renamed["alias"], "renamed_tool_data");

    let analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: user.clone(),
            session_id: session.id,
            request_message_id: None,
            intent: serde_json::json!({"goal":"copy"}),
            script: "result = limit(1, source);".into(),
            inputs: vec![AnalysisInput {
                source: AnalysisInputSource::DataResourceId(resource_id),
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            skill_refs: vec![],
        })
        .await
        .unwrap();
    let view_count = diesel::sql_query("SELECT count(*) count FROM dataset_views")
        .get_result::<common::CountRow>(&mut app.pool.get().unwrap())
        .unwrap()
        .count;
    assert_eq!(view_count, 0, "analysis must not create chart views");
    let promoted = tools::promote_result_dataset::PromoteResultDatasetTool::new(tool_services)
        .call(
            &mut context,
            tools::promote_result_dataset::Args {
                dataset_id: analysis.datasets[0].id,
                display_name: "提升结果".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(promoted["sourceKind"], "analysis");
    assert_eq!(promoted["lineage"]["datasetId"], analysis.datasets[0].id);
}

#[test]
fn manifest_allowed_tools_equal_runtime_registry() {
    let knowledge = AgentKnowledge::load_embedded("v4.5.0").unwrap();
    let mut manifest = knowledge.manifest.allowed_tools.clone();
    let mut runtime = tools::tool_implementation_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    manifest.sort();
    runtime.sort();

    assert_eq!(
        runtime.len(),
        24,
        "runtime registry must contain 24 unique tools"
    );
    runtime.dedup();
    assert_eq!(runtime.len(), 24, "runtime tool names must be unique");
    assert!(!runtime.iter().any(|tool| tool == "create_dataset_view"));
    assert!(runtime.iter().any(|tool| tool == "publish_model_version"));
    assert!(runtime.iter().any(|tool| tool == "update_study_results"));
    assert!(!runtime.iter().any(|tool| tool == "create_script"));
    assert_eq!(
        manifest, runtime,
        "manifest and runtime tool registries must be bidirectionally equal"
    );
}

#[test]
fn tool_catalog_rejects_a_manifest_tool_without_an_implementation() {
    let mut knowledge = AgentKnowledge::load_embedded("v4.5.0").unwrap();
    knowledge
        .manifest
        .allowed_tools
        .push("missing_implementation".into());

    let error = tools::validate_tool_catalog(&knowledge.manifest).unwrap_err();

    assert_eq!(error.code(), "agent_tool_implementation_missing");
}

#[tokio::test]
async fn tool_registration_follows_manifest_order() {
    let app = TestApp::spawn().await;
    let mut knowledge = (*app.state.agent_knowledge).clone();
    knowledge.manifest.allowed_tools.reverse();
    let expected = knowledge.manifest.allowed_tools.clone();
    let mut services = services(&app, principal("alice"));
    Arc::get_mut(&mut services).unwrap().agent_knowledge = Arc::new(knowledge);

    let actual = tools::build_tools(services)
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
}

#[tokio::test]
async fn run_result_analysis_has_no_chart_creation_input() {
    let app = TestApp::spawn().await;
    let tool =
        tools::run_result_analysis::RunResultAnalysisTool::new(services(&app, principal("alice")));
    let parameters = tool.parameters();
    assert!(parameters["properties"].get("initialViews").is_none());
    assert!(
        parameters["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "inputs")
    );
    assert_eq!(
        parameters["properties"]["inputs"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        parameters["properties"]["inputs"]["items"]["properties"]
            .get("modelDataframeId")
            .is_some()
    );
    assert_eq!(parameters["properties"]["intent"]["type"], "string");
    assert_eq!(parameters["properties"]["intent"]["minLength"], 1);
    assert_eq!(parameters["properties"]["intent"]["maxLength"], 1_000);
    assert!(
        serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent":"join the selected result datasets",
            "script":"joined = join(left, right)",
            "inputs":[
                {"datasetId":1,"alias":"left"},
                {"datasetId":2,"alias":"right"}
            ],
            "expectedOutputs":["joined"]
        }))
        .is_ok(),
        "explicit multi-input analysis must remain supported"
    );
    assert!(
        serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent":{"request":"join the selected result datasets"},
            "script":"joined = join(left, right)",
            "inputs":[
                {"datasetId":1,"alias":"left"},
                {"datasetId":2,"alias":"right"}
            ],
            "expectedOutputs":["joined"]
        }))
        .is_err(),
        "object-valued intent must be rejected at the tool boundary"
    );
    assert!(
        serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent":"copy the selected result",
            "script":"result = source",
            "inputs":[],
            "expectedOutputs":["result"],
            "initialViews":{}
        }))
        .is_err()
    );
}

#[tokio::test]
async fn read_model_dataframe_does_not_require_a_draft_ref_for_active_scope() {
    let app = TestApp::spawn().await;
    let tool = tools::read_model_dataframe::ReadModelDataFrameTool::new(services(
        &app,
        principal("alice"),
    ));
    let parameters = tool.parameters();
    let required = parameters["required"].as_array().unwrap();

    assert!(required.iter().any(|value| value == "scope"));
    assert!(required.iter().any(|value| value == "tableName"));
    assert!(
        !required.iter().any(|value| value == "expectedDraft"),
        "expectedDraft is conditional: active omits it and draft is validated by the service"
    );
    assert!(
        serde_json::from_value::<tools::read_model_dataframe::Args>(serde_json::json!({
            "scope":"active",
            "tableName":"bus",
            "columns":["id"],
            "equals":{},
            "limit":20
        }))
        .is_ok()
    );
}

#[tokio::test]
async fn run_result_analysis_accepts_bounded_sources_and_freezes_mixed_input_snapshots() {
    let engine = ScenarioEngine::default();
    for _ in 0..4 {
        engine.enqueue(EngineScenario::Success(vec![DatasetFixture {
            name: "result".into(),
            rows: vec![("one".into(), 1.0)],
            value_unit: None,
        }]));
    }
    let app = TestApp::builder().engine(Arc::new(engine)).spawn().await;
    let user = principal("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "analysis source tool", 1)
        .await
        .unwrap();
    let historical = publish_active_version(
        &app,
        &user,
        session.id,
        "historical-source",
        b"id,name\n1,old\n",
    )
    .await;
    let active = publish_active_version(
        &app,
        &user,
        session.id,
        "active-source",
        b"id,name\n1,new\n",
    )
    .await;
    let model_dataframe_id = active.dataframes[0].id;
    let dataset_id = insert_direct_dataset(&app, session.id, &active, "active-source");
    let resource_id = publish_tool_resource(&app, &user, session.id).await;
    let mut tool_services = services_for_session(&app, user, session.id);
    Arc::get_mut(&mut tool_services).unwrap().request_message_id =
        insert_request_message(&app, session.id);
    tool_services
        .loaded_skills
        .lock()
        .await
        .insert("result-analysis".into());
    let analysis_attempts = tool_services.analysis_attempts.clone();
    let tool = tools::run_result_analysis::RunResultAnalysisTool::new(tool_services);

    for input in [
        serde_json::json!({"modelDataframeId": model_dataframe_id, "alias": "model"}),
        serde_json::json!({"datasetId": dataset_id, "alias": "source"}),
        serde_json::json!({"resourceId": resource_id, "alias": "local"}),
    ] {
        *analysis_attempts.lock().await = 0;
        let args = serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent": "single bounded source",
            "script": format!("result = limit(1, {});", input["alias"].as_str().unwrap()),
            "inputs": [input],
            "expectedOutputs": ["result"]
        }))
        .unwrap();
        let value = tool.call(&mut ToolContext::new(), args).await.unwrap();
        assert_eq!(value["status"], "succeeded");
    }

    for invalid_inputs in [
        serde_json::json!([{"alias":"missing"}]),
        serde_json::json!([{
            "datasetId": dataset_id,
            "resourceId": resource_id,
            "alias":"ambiguous"
        }]),
    ] {
        let args = serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent":"invalid source selection",
            "script":"result = limit(1, missing);",
            "inputs":invalid_inputs,
            "expectedOutputs":["result"]
        }))
        .unwrap();
        let error = tool.call(&mut ToolContext::new(), args).await.unwrap_err();
        assert_eq!(error.code, "analysis_inputs_invalid");
    }

    let too_many = (0..21)
        .map(|index| {
            serde_json::json!({
                "modelDataframeId": model_dataframe_id,
                "alias": format!("model_{index}")
            })
        })
        .collect::<Vec<_>>();
    for (inputs, expected_code) in [
        (too_many, "analysis_inputs_invalid"),
        (
            vec![
                serde_json::json!({"modelDataframeId":model_dataframe_id,"alias":"same"}),
                serde_json::json!({"resourceId":resource_id,"alias":"same"}),
            ],
            "analysis_alias_invalid",
        ),
        (
            vec![serde_json::json!({
                "modelDataframeId": historical.dataframes[0].id,
                "alias":"stale"
            })],
            "analysis_input_not_found",
        ),
        (
            vec![serde_json::json!({
                "modelDataframeId":model_dataframe_id,
                "alias":"select"
            })],
            "analysis_alias_invalid",
        ),
    ] {
        *analysis_attempts.lock().await = 0;
        let args = serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent":"reject invalid bounded inputs",
            "script":"result = limit(1, select);",
            "inputs":inputs,
            "expectedOutputs":["result"]
        }))
        .unwrap();
        let error = tool.call(&mut ToolContext::new(), args).await.unwrap_err();
        assert_eq!(error.code, expected_code);
    }

    *analysis_attempts.lock().await = 0;
    let mixed = serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
        "intent":"combine active model, result dataset, and local resource",
        "script":"result = limit(1, model);",
        "inputs":[
            {"modelDataframeId":model_dataframe_id,"alias":"model"},
            {"datasetId":dataset_id,"alias":"simulation"},
            {"resourceId":resource_id,"alias":"local"}
        ],
        "expectedOutputs":["result"]
    }))
    .unwrap();
    let value = tool
        .call(&mut ToolContext::new(), mixed)
        .await
        .expect("matching mixed sources must run");
    assert_eq!(value["status"], "succeeded");
    let analysis_run_id = value["analysisRunId"]
        .as_str()
        .unwrap()
        .parse::<i32>()
        .unwrap();
    let snapshots = diesel::sql_query("SELECT source_kind,dataset_id,resource_id,model_dataframe_id,source_schema_json,source_schema_hash,source_content_hash FROM result_analysis_inputs WHERE analysis_run_id=?1 ORDER BY ordinal")
        .bind::<diesel::sql_types::Integer, _>(analysis_run_id)
        .load::<AnalysisInputSnapshotRow>(&mut app.pool.get().unwrap())
        .unwrap();
    assert_eq!(snapshots.len(), 3);
    assert_eq!(snapshots[0].source_kind, "active_model");
    assert_eq!(snapshots[0].model_dataframe_id, Some(model_dataframe_id));
    assert_eq!(
        snapshots[0].source_schema_json,
        active.dataframes[0].schema_json
    );
    assert_eq!(
        snapshots[0].source_content_hash,
        active.dataframes[0].content_hash
    );
    assert_eq!(snapshots[1].source_kind, "dataset");
    assert_eq!(snapshots[1].dataset_id, Some(dataset_id));
    assert!(!snapshots[1].source_schema_hash.is_empty());
    assert!(!snapshots[1].source_content_hash.is_empty());
    assert_eq!(snapshots[2].source_kind, "data_resource");
    assert_eq!(
        snapshots[2].resource_id.as_deref(),
        Some(resource_id.as_str())
    );
    assert!(!snapshots[2].source_schema_hash.is_empty());
    assert!(!snapshots[2].source_content_hash.is_empty());
}

#[tokio::test]
async fn analysis_repair_budget_allows_four_service_calls_and_rejects_the_fifth() {
    let engine = ScenarioEngine::default();
    for _ in 0..4 {
        engine.enqueue(EngineScenario::Failure {
            code: "model_error".into(),
            stderr: b"repairable parse failure".to_vec(),
            exit_code: 7,
        });
    }
    let app = TestApp::builder()
        .engine(Arc::new(engine.clone()))
        .spawn()
        .await;
    let user = principal("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "analysis budget", 1)
        .await
        .unwrap();
    let resource_id = publish_tool_resource(&app, &user, session.id).await;
    let mut tool_services = services_for_session(&app, user, session.id);
    Arc::get_mut(&mut tool_services).unwrap().request_message_id =
        insert_request_message(&app, session.id);
    tool_services
        .loaded_skills
        .lock()
        .await
        .insert("result-analysis".into());
    let tool = tools::run_result_analysis::RunResultAnalysisTool::new(tool_services);

    for attempt in 1..=5 {
        let args = serde_json::from_value::<tools::run_result_analysis::Args>(serde_json::json!({
            "intent":format!("repair attempt {attempt}"),
            "script":"result = limit(1, source);",
            "inputs":[{"resourceId":resource_id,"alias":"source"}],
            "expectedOutputs":["result"]
        }))
        .unwrap();
        let error = tool.call(&mut ToolContext::new(), args).await.unwrap_err();
        assert_eq!(
            error.code,
            if attempt <= 4 {
                "analysis_engine_failed"
            } else {
                "analysis_repair_budget_exhausted"
            }
        );
    }

    assert_eq!(engine.calls().len(), 4);
    let runs =
        diesel::sql_query("SELECT count(*) count FROM result_analysis_runs WHERE session_id=?1")
            .bind::<diesel::sql_types::Integer, _>(session.id)
            .get_result::<common::CountRow>(&mut app.pool.get().unwrap())
            .unwrap();
    assert_eq!(runs.count, 4);
}

#[tokio::test]
async fn read_result_dataset_is_bounded_projected_paginated_and_session_scoped() {
    let engine = ScenarioEngine::default();
    engine.enqueue(EngineScenario::Success(vec![DatasetFixture {
        name: "result".into(),
        rows: vec![
            ("one".into(), 1.0),
            ("two".into(), 2.0),
            ("three".into(), 3.0),
        ],
        value_unit: None,
    }]));
    let app = TestApp::builder().engine(Arc::new(engine)).spawn().await;
    let user = principal("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "result read", 1)
        .await
        .unwrap();
    let other_session = app
        .state
        .session_repo
        .create(&user.id, "other result read", 1)
        .await
        .unwrap();
    let resource_id = publish_tool_resource(&app, &user, session.id).await;
    let analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: user.clone(),
            session_id: session.id,
            request_message_id: None,
            intent: serde_json::json!("read result"),
            script: "result = limit(3, source);".into(),
            inputs: vec![AnalysisInput {
                source: AnalysisInputSource::DataResourceId(resource_id),
                alias: "source".into(),
            }],
            expected_outputs: vec!["result".into()],
            skill_refs: vec![],
        })
        .await
        .unwrap();
    let dataset_id = analysis.datasets[0].id;
    let services = services_for_session(&app, user.clone(), session.id);
    services
        .loaded_skills
        .lock()
        .await
        .insert("result-analysis".into());
    let tool = tools::read_result_dataset::ReadResultDatasetTool::new(services);
    let first = tool
        .call(
            &mut ToolContext::new(),
            tools::read_result_dataset::Args {
                dataset_id,
                columns: vec!["value".into()],
                limit: Some(1),
                cursor: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(first["metadata"]["id"], dataset_id.to_string());
    assert_eq!(first["page"]["columns"], serde_json::json!(["value"]));
    assert_eq!(first["page"]["rows"].as_array().unwrap().len(), 1);
    let cursor = first["page"]["nextCursor"].as_str().unwrap().to_string();
    let second = tool
        .call(
            &mut ToolContext::new(),
            tools::read_result_dataset::Args {
                dataset_id,
                columns: vec!["value".into()],
                limit: Some(1),
                cursor: Some(cursor),
            },
        )
        .await
        .unwrap();
    assert_eq!(second["page"]["rows"].as_array().unwrap().len(), 1);

    for (columns, limit) in [(vec!["x".to_string(); 33], Some(1)), (vec![], Some(101))] {
        let error = tool
            .call(
                &mut ToolContext::new(),
                tools::read_result_dataset::Args {
                    dataset_id,
                    columns,
                    limit,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_dataset_page");
    }

    for isolated in [
        services_for_session(&app, user, other_session.id),
        services_for_session(&app, principal("bob@example.com"), session.id),
    ] {
        isolated
            .loaded_skills
            .lock()
            .await
            .insert("result-analysis".into());
        let error = tools::read_result_dataset::ReadResultDatasetTool::new(isolated)
            .call(
                &mut ToolContext::new(),
                tools::read_result_dataset::Args {
                    dataset_id,
                    columns: vec![],
                    limit: Some(1),
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "dataset_not_found");
    }
}

#[tokio::test]
async fn invalid_analysis_intent_fails_before_persistent_side_effects() {
    let app = TestApp::spawn().await;
    let registered = tools::build_tools(services(&app, principal("alice")));
    let tool = registered
        .iter()
        .find(|tool| tool.name() == "run_result_analysis")
        .expect("run_result_analysis registered");
    let before_runs = diesel::sql_query("SELECT count(*) count FROM result_analysis_runs")
        .get_result::<common::CountRow>(&mut app.pool.get().unwrap())
        .unwrap()
        .count;
    let before_artifacts = diesel::sql_query("SELECT count(*) count FROM artifacts")
        .get_result::<common::CountRow>(&mut app.pool.get().unwrap())
        .unwrap()
        .count;

    for intent in ["   \n\t".to_string(), "压".repeat(1_001)] {
        let result = tool
            .execute(
                serde_json::json!({
                    "intent": intent,
                    "script": "result = select(col(time), source);",
                    "inputs": [{"datasetId": 1, "alias": "source"}],
                    "expectedOutputs": ["result"]
                })
                .to_string(),
                &mut ToolContext::new(),
            )
            .await;
        let error = result.error().expect("invalid intent must fail");
        assert_eq!(error.code(), Some("analysis_intent_invalid"));
        assert!(
            error
                .model_feedback()
                .is_some_and(|feedback| feedback.contains("analysis_intent_invalid"))
        );
    }

    let after_runs = diesel::sql_query("SELECT count(*) count FROM result_analysis_runs")
        .get_result::<common::CountRow>(&mut app.pool.get().unwrap())
        .unwrap()
        .count;
    let after_artifacts = diesel::sql_query("SELECT count(*) count FROM artifacts")
        .get_result::<common::CountRow>(&mut app.pool.get().unwrap())
        .unwrap()
        .count;
    assert_eq!(after_runs, before_runs);
    assert_eq!(after_artifacts, before_artifacts);
}

#[tokio::test]
async fn registered_tools_preserve_domain_error_feedback_and_metadata() {
    let app = TestApp::spawn().await;
    let registered = tools::build_tools(services(&app, principal("alice")));
    let planner = registered
        .iter()
        .find(|tool| tool.name() == "search_study_facades")
        .expect("search_study_facades registered");
    let result = planner
        .execute(
            r#"{"query":"潮流","explicitFunction":null}"#.into(),
            &mut ToolContext::new(),
        )
        .await;
    let error = result.error().expect("missing skill should fail");

    assert_eq!(error.code(), Some("skill_required"));
    assert_eq!(error.retryable(), Some(false));
    assert_ne!(error.model_feedback(), Some("the tool failed"));
    assert!(
        error
            .model_feedback()
            .is_some_and(|feedback| feedback.contains("skill_required"))
    );
}

#[tokio::test]
async fn registered_analysis_tool_returns_sanitized_structured_repair_feedback() {
    let engine = ScenarioEngine::default();
    let mut stderr = b"parse failed at /private/secret/input.rss TOKEN=do-not-leak".to_vec();
    stderr.extend(vec![b'\n'; 70 * 1024]);
    engine.enqueue(EngineScenario::Failure {
        code: "model_error".into(),
        stderr,
        exit_code: 7,
    });
    let app = TestApp::builder().engine(Arc::new(engine)).spawn().await;
    let user = principal("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "diagnostic tool", 1)
        .await
        .unwrap();
    let resource_id = publish_tool_resource(&app, &user, session.id).await;
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("INSERT INTO messages(session_id,role,kind,content,created_at_ms) VALUES(?1,'assistant','tool_call','analysis',1)")
        .bind::<diesel::sql_types::Integer, _>(session.id)
        .execute(&mut conn)
        .unwrap();
    let request_message_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result::<i32>(&mut conn)
    .unwrap();
    drop(conn);
    let mut services = services_for_session(&app, user, session.id);
    Arc::get_mut(&mut services).unwrap().request_message_id = request_message_id;
    services
        .loaded_skills
        .lock()
        .await
        .insert("result-analysis".into());
    let registered = tools::build_tools(services);
    let tool = registered
        .iter()
        .find(|tool| tool.name() == "run_result_analysis")
        .unwrap();

    let result = tool
        .execute(
            serde_json::json!({
                "intent": "read resource",
                "script": "result = limit(1, source);",
                "inputs": [{"resourceId": resource_id, "alias": "source"}],
                "expectedOutputs": ["result"]
            })
            .to_string(),
            &mut ToolContext::new(),
        )
        .await;
    let error = result
        .error()
        .expect("analysis engine failure must surface");
    let feedback = error.model_feedback().expect("structured model feedback");
    assert_eq!(error.code(), Some("analysis_engine_failed"));
    assert!(feedback.contains("\"repairable\":true"));
    assert!(feedback.contains("\"stage\":\"run\""));
    assert!(!feedback.contains("/private/secret"));
    assert!(!feedback.contains("do-not-leak"));
    assert!(feedback.len() <= 64 * 1024);
}

#[tokio::test]
async fn get_storage_usage_returns_current_scope_only() {
    let app = TestApp::spawn().await;
    let artifacts = &app.state.artifact_service;
    for (scope_id, bytes) in [(2, b"alice".as_slice()), (3, b"bob".as_slice())] {
        let scope = artifacts.resolve_scope(scope_id).await.unwrap();
        let staged = artifacts
            .store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .unwrap();
        artifacts
            .commit(
                scope.id,
                staged,
                ArtifactMetadata {
                    format: "bin".into(),
                    media_type: Some("application/octet-stream".into()),
                },
            )
            .await
            .unwrap();
    }

    let mut context = ToolContext::new();
    let value =
        tools::get_storage_usage::GetStorageUsageTool::new(services(&app, principal("alice")))
            .call(&mut context, tools::get_storage_usage::Args {})
            .await
            .unwrap();

    assert_eq!(value["scope"], "user");
    assert_eq!(value["artifactCount"], 1);
    assert_eq!(value["logicalBytes"], 5);
    assert_eq!(value["artifactLimit"], 20_000);
    assert_eq!(value["byteLimit"], 20_i64 * 1024 * 1024 * 1024);
    assert_eq!(value["warningRatio"], 0.8);
    assert_eq!(value["isWarning"], false);
    assert_eq!(value["isExceeded"], false);
}
