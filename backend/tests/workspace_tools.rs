mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::sync::Arc;

use backend::adapters::StubEngine;
use backend::agent::prompt::PromptAssembler;
use backend::application::{
    ArtifactMetadata, DataframeBindingDraft, ModelChange, ModelPatch, ModelVersionStatus,
    SourceKind, VersionDraft, VersionInputDraft, VersionOrigin,
};
use backend::auth::{AccessScope, Principal, StorageScopeRef};
use backend::db::models::NewSimulationTask;
use backend::storage::StageLimits;
use backend::tools::{self, ToolServices};
use rig::tool::{Tool, ToolContext};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore};

use common::TestApp;

fn principal() -> Principal {
    Principal {
        access: AccessScope {
            kind: "instance".into(),
            key: "instance".into(),
        },
        storage: StorageScopeRef {
            kind: "instance".into(),
            key: "instance".into(),
        },
        subject: "instance".into(),
    }
}

async fn workspace_with_csv(app: &TestApp, csvs: &[(&str, &[u8])]) -> (i32, i32) {
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.access, "tools", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&p.access, session.id)
        .await
        .unwrap();
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&p.storage)
        .await
        .unwrap();
    let mut inputs = Vec::new();
    for (index, (name, bytes)) in csvs.iter().enumerate() {
        let staged = app
            .state
            .artifact_service
            .store()
            .stage(Cursor::new(bytes.to_vec()), StageLimits::default())
            .unwrap();
        let artifact = app
            .state
            .artifact_service
            .commit(
                scope.id,
                staged,
                ArtifactMetadata {
                    format: "csv".into(),
                    media_type: Some("text/csv".into()),
                },
            )
            .await
            .unwrap();
        inputs.push(VersionInputDraft {
            artifact_id: artifact.id,
            sha256: artifact.sha256,
            role: "input".into(),
            relative_path: Some(format!("tables/{name}")),
            display_name: name.to_string(),
            source_artifact_id: None,
            source_artifact_sha256: None,
            sheet_name: None,
            binding: Some(DataframeBindingDraft {
                table_kind: Some("bus".into()),
                confidence: Some(1.0),
                resolution: if index == 0 {
                    "inferred".into()
                } else {
                    "pending".into()
                },
                columns: json!([{"name":"id","dtype":"string"},{"name":"p","dtype":"float64"}]),
                evidence: json!({"candidates":[{"tableKind":"bus"}],"marker":"preserve-me"}),
            }),
        });
    }
    let version = app
        .state
        .workspace_service
        .publish_version(
            &p.access,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Dataframes,
                origin: VersionOrigin::Upload,
                manifest: json!({"missing":[],"diagnostics":[]}),
                patch: None,
                inputs,
            },
        )
        .await
        .unwrap();
    (session.id, version.id)
}

async fn example_workspace_with_rss(app: &TestApp, bytes: &[u8]) -> (i32, i32, i32) {
    let p = principal();
    let session = app
        .state
        .session_repo
        .create(&p.access, "example patch", 1)
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get_or_create(&p.access, session.id)
        .await
        .unwrap();
    let scope = app
        .state
        .artifact_service
        .resolve_scope(&p.storage)
        .await
        .unwrap();
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(bytes.to_vec()), StageLimits::default())
        .unwrap();
    let artifact = app
        .state
        .artifact_service
        .commit(
            scope.id,
            staged,
            ArtifactMetadata {
                format: "rss".into(),
                media_type: Some("text/plain".into()),
            },
        )
        .await
        .unwrap();
    let version = app
        .state
        .workspace_service
        .publish_version(
            &p.access,
            workspace.workspace.id,
            None,
            VersionDraft {
                status: ModelVersionStatus::Ready,
                source_kind: SourceKind::Rss,
                origin: VersionOrigin::Example {
                    example_id: "ex_fixture".into(),
                    engine_resource_snapshot: "snapshot".into(),
                    source_tree_hash: "tree".into(),
                },
                manifest: json!({"entryPath":"main.rss","missing":[],"diagnostics":[]}),
                patch: None,
                inputs: vec![VersionInputDraft {
                    artifact_id: artifact.id,
                    sha256: artifact.sha256,
                    role: "entry".into(),
                    relative_path: Some("main.rss".into()),
                    display_name: "main.rss".into(),
                    source_artifact_id: None,
                    source_artifact_sha256: None,
                    sheet_name: None,
                    binding: None,
                }],
            },
        )
        .await
        .unwrap();
    let input_id = app
        .state
        .workspace_service
        .inputs(&p.access, version.id)
        .await
        .unwrap()[0]
        .0
        .id;
    (session.id, version.id, input_id)
}

fn services(app: &TestApp, session_id: i32) -> Arc<ToolServices> {
    let p = principal();
    Arc::new(ToolServices {
        engine: Arc::new(StubEngine),
        writer: backend::application::EventWriter::new(
            p.access.clone(),
            session_id,
            app.state.message_repo.clone(),
            app.state.events.clone(),
        ),
        session_id,
        request_message_id: 0,
        task_repo: app.state.task_repo.clone(),
        semaphore: Arc::new(Semaphore::new(1)),
        scope: p.access.clone(),
        principal: p,
        workspaces: app.state.workspace_service.clone(),
        example_import: app.state.example_import_service.clone(),
        model_patch: app.state.model_patch_service.clone(),
        engine_snapshot: app.state.engine_snapshot.clone(),
        agent_knowledge: app.state.agent_knowledge.clone(),
        loaded_skills: Arc::new(Mutex::new(BTreeSet::new())),
        simulations: app.state.simulation_service.clone(),
        datasets: app.state.dataset_service.clone(),
        analysis: app.state.analysis_service.clone(),
        comparisons: app.state.comparison_service.clone(),
        history: app.state.history_service.clone(),
        artifact_service: app.state.artifact_service.clone(),
    })
}

#[tokio::test]
async fn inspect_workspace_exposes_stable_input_display_names_to_the_agent() {
    let app = TestApp::spawn().await;
    let (session_id, _) = workspace_with_csv(&app, &[("bus-upload.csv", b"id,p\nb1,100\n")]).await;
    let result = tools::inspect_workspace::InspectWorkspaceTool::new(services(&app, session_id))
        .call(&mut ToolContext::new(), tools::inspect_workspace::Args {})
        .await
        .unwrap();

    assert_eq!(
        result["versions"][0]["inputs"][0]["fileName"],
        "bus-upload.csv"
    );
    assert_eq!(
        result["versions"][0]["inputs"][0]["relativePath"],
        "tables/bus-upload.csv"
    );
    assert!(result["versions"][0]["inputs"][0]["sheetName"].is_null());
}

#[tokio::test]
async fn activate_tool_explains_that_expected_draft_is_the_current_candidate() {
    let app = TestApp::spawn().await;
    let (session_id, _) = workspace_with_csv(&app, &[("bus-upload.csv", b"id,p\nb1,100\n")]).await;
    let description =
        tools::activate_model_version::ActivateModelVersionTool::new(services(&app, session_id))
            .description();

    assert!(description.contains("expectedDraftVersionId"));
    assert!(description.contains("versionId 相同"));
}

#[tokio::test]
async fn modify_tool_documents_the_reserved_dss_property_patch_shape() {
    let app = TestApp::spawn().await;
    let (session_id, _) = workspace_with_csv(&app, &[("bus-upload.csv", b"id,p\nb1,100\n")]).await;
    let description =
        tools::modify_model::ModifyModelTool::new(services(&app, session_id)).description();

    assert!(description.contains("dss_property 不是引擎函数"));
    assert!(description.contains("rowKey.object"));
    assert!(description.contains("changes.<property>"));
}

#[tokio::test]
async fn prompt_skill_document_and_example_tools_are_id_bounded() {
    let app = TestApp::spawn().await;
    let (session_id, _) = workspace_with_csv(&app, &[("bus.csv", b"id,p\nb1,100\n")]).await;
    let prompt = PromptAssembler::assemble(&app.state.agent_knowledge, &app.state.engine_snapshot);
    assert!(prompt.contains("model-intake"));
    assert!(prompt.contains(&app.state.engine_snapshot.id));
    assert!(
        !prompt.contains("# 模型输入整理"),
        "full skill body must be loaded on demand"
    );

    let services = services(&app, session_id);
    let mut context = ToolContext::new();
    let loaded = tools::load_agent_skill::LoadAgentSkillTool::new(services.clone())
        .call(
            &mut context,
            tools::load_agent_skill::Args {
                skill_id: "model-intake".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(loaded["id"], "model-intake");
    assert!(
        loaded["version"].is_string()
            && loaded["sha256"].is_string()
            && loaded["content"].is_string()
    );
    assert!(services.loaded_skills.lock().await.contains("model-intake"));
    let unknown = tools::load_agent_skill::LoadAgentSkillTool::new(services.clone())
        .call(
            &mut context,
            tools::load_agent_skill::Args {
                skill_id: "../secret".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(unknown.code, "skill_not_found");
    let script_patch = ModelPatch {
        table_kind: "rss_parameter".into(),
        row_key: BTreeMap::from([("name".into(), json!("step"))]),
        changes: BTreeMap::from([(
            "value".into(),
            ModelChange {
                from: json!(60),
                to: json!(900),
                unit: "s".into(),
            },
        )]),
    };
    let skill_required = tools::modify_model::ModifyModelTool::new(services.clone())
        .call(
            &mut context,
            tools::modify_model::Args {
                input_id: 1,
                patch: script_patch,
                expected_draft_version_id: Some(1),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(skill_required.code, "skill_required");

    let section = &app.state.engine_snapshot.knowledge.sections[0];
    let read = tools::read_engine_doc_section::ReadEngineDocSectionTool::new(services.clone())
        .call(
            &mut context,
            tools::read_engine_doc_section::Args {
                doc_id: section.doc_id.clone(),
                section_id: section.section_id.clone(),
                max_chars: Some(7),
            },
        )
        .await
        .unwrap();
    assert!(read["content"].as_str().unwrap().chars().count() <= 7);
    let wrong = tools::read_engine_doc_section::ReadEngineDocSectionTool::new(services.clone())
        .call(
            &mut context,
            tools::read_engine_doc_section::Args {
                doc_id: "docs/guide.md".into(),
                section_id: section.section_id.clone(),
                max_chars: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(wrong.code, "doc_section_not_found");
    let example_id = app.state.engine_snapshot.examples[0].id.clone();
    let inspected = tools::inspect_example::InspectExampleTool::new(services.clone())
        .call(&mut context, tools::inspect_example::Args { example_id })
        .await
        .unwrap();
    assert!(inspected["example"]["id"].is_string());

    let schemas = [
        <tools::replace_model_input::ReplaceModelInputTool as Tool>::parameters(
            &tools::replace_model_input::ReplaceModelInputTool::new(services.clone()),
        ),
        <tools::remove_model_input::RemoveModelInputTool as Tool>::parameters(
            &tools::remove_model_input::RemoveModelInputTool::new(services),
        ),
    ];
    for schema in schemas {
        let text = schema.to_string().to_lowercase();
        assert!(!text.contains("filename") && !text.contains("path"));
        assert!(text.contains("expecteddraftversionid"));
    }
}

#[tokio::test]
async fn mapping_replace_remove_and_concurrency_create_only_child_versions() {
    let app = TestApp::spawn().await;
    let (session_id, base_id) = workspace_with_csv(
        &app,
        &[
            ("old.csv", b"id,p\nb1,100\n"),
            ("new.csv", b"id,p\nb1,120\n"),
        ],
    )
    .await;
    let p = principal();
    let initial = app
        .state
        .workspace_service
        .inputs(&p.access, base_id)
        .await
        .unwrap();
    let old_id = initial[0].0.id;
    let missing = app
        .state
        .model_patch_service
        .confirm_mapping(&p, session_id, old_id, "bus".into(), None)
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "expected_version_required");
    assert_eq!(
        app.state
            .workspace_service
            .list_versions(&p.access, session_id)
            .await
            .unwrap()
            .len(),
        1
    );
    let invalid_mapping = app
        .state
        .model_patch_service
        .confirm_mapping(
            &p,
            session_id,
            old_id,
            "unknown-table".into(),
            Some(base_id),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid_mapping.code(), "mapping_invalid");
    assert_eq!(
        app.state
            .workspace_service
            .list_versions(&p.access, session_id)
            .await
            .unwrap()
            .len(),
        1
    );

    let confirmed = app
        .state
        .model_patch_service
        .confirm_mapping(&p, session_id, old_id, "bus".into(), Some(base_id))
        .await
        .unwrap();
    let confirmed_inputs = app
        .state
        .workspace_service
        .inputs(&p.access, confirmed.id)
        .await
        .unwrap();
    let confirmed_old = confirmed_inputs
        .iter()
        .find(|(input, _)| input.display_name == "old.csv")
        .unwrap();
    assert_eq!(
        confirmed_old.1.as_ref().unwrap().resolution,
        "user_confirmed"
    );
    assert!(
        confirmed_old
            .1
            .as_ref()
            .unwrap()
            .evidence_json
            .contains("preserve-me")
    );

    let stale = app
        .state
        .model_patch_service
        .remove_input(&p, session_id, confirmed_old.0.id, Some(base_id))
        .await
        .unwrap_err();
    assert_eq!(stale.code(), "version_conflict");
    let ids: Vec<_> = confirmed_inputs
        .iter()
        .map(|(input, _)| (input.display_name.clone(), input.id))
        .collect();
    let old = ids.iter().find(|(name, _)| name == "old.csv").unwrap().1;
    let new = ids.iter().find(|(name, _)| name == "new.csv").unwrap().1;
    let replaced = app
        .state
        .model_patch_service
        .replace_input(&p, session_id, old, new, Some(confirmed.id))
        .await
        .unwrap();
    let replaced_inputs = app
        .state
        .workspace_service
        .inputs(&p.access, replaced.id)
        .await
        .unwrap();
    assert_eq!(replaced_inputs.len(), 1);
    assert_eq!(replaced_inputs[0].0.display_name, "new.csv");

    let third = workspace_with_csv(&app, &[("unrelated.csv", b"id,p\nu1,1\n")]).await;
    assert_ne!(third.0, session_id);
    let removed_error = app
        .state
        .model_patch_service
        .remove_input(&p, session_id, replaced_inputs[0].0.id, Some(replaced.id))
        .await
        .unwrap_err();
    assert_eq!(removed_error.code(), "model_empty");
}

#[tokio::test]
async fn structured_patch_fails_closed_then_writes_a_new_parquet_child() {
    let app = TestApp::spawn().await;
    let (session_id, base_id) =
        workspace_with_csv(&app, &[("bus.csv", b"id,p\nb1,100\nb2,100\n")]).await;
    let p = principal();
    let input_id = app
        .state
        .workspace_service
        .inputs(&p.access, base_id)
        .await
        .unwrap()[0]
        .0
        .id;
    let count = || async {
        app.state
            .workspace_service
            .list_versions(&p.access, session_id)
            .await
            .unwrap()
            .len()
    };
    let patch = |row_key: Value, from: Value, unit: &str| ModelPatch {
        table_kind: "bus".into(),
        row_key: serde_json::from_value(row_key).unwrap(),
        changes: BTreeMap::from([(
            "p".into(),
            ModelChange {
                from,
                to: json!(120),
                unit: unit.into(),
            },
        )]),
    };
    for (candidate, code) in [
        (patch(json!({"p":100}), json!(100), "W"), "row_not_unique"),
        (
            patch(json!({"id":"b1"}), json!(99), "W"),
            "old_value_mismatch",
        ),
        (
            patch(json!({"id":"b1"}), json!(100), "V"),
            "unit_incompatible",
        ),
    ] {
        let error = app
            .state
            .model_patch_service
            .modify(&p, session_id, input_id, candidate, Some(base_id))
            .await
            .unwrap_err();
        assert_eq!(error.code(), code);
        assert_eq!(count().await, 1);
    }
    let version = app
        .state
        .model_patch_service
        .modify(
            &p,
            session_id,
            input_id,
            patch(json!({"id":"b1"}), json!(100), "W"),
            Some(base_id),
        )
        .await
        .unwrap();
    assert_eq!(version.parent_version_id, Some(base_id));
    assert!(version.patch_json.is_some());
    let inputs = app
        .state
        .workspace_service
        .inputs(&p.access, version.id)
        .await
        .unwrap();
    assert!(inputs[0].0.display_name.ends_with(".parquet"));
    assert_eq!(count().await, 2);
}

#[tokio::test]
async fn imported_rss_parameter_patch_creates_user_child_and_ambiguous_text_fails_closed() {
    let app = TestApp::spawn().await;
    let p = principal();
    let (session_id, base_id, input_id) =
        example_workspace_with_rss(&app, b"step = 60;\nrun = true;\n").await;
    let patch = ModelPatch {
        table_kind: "rss_parameter".into(),
        row_key: BTreeMap::from([("name".into(), json!("step"))]),
        changes: BTreeMap::from([(
            "value".into(),
            ModelChange {
                from: json!(60),
                to: json!(900),
                unit: "s".into(),
            },
        )]),
    };
    let child = app
        .state
        .model_patch_service
        .modify(&p, session_id, input_id, patch, Some(base_id))
        .await
        .unwrap();
    assert_eq!(child.parent_version_id, Some(base_id));
    assert_eq!(child.origin_kind, "generated");
    assert_eq!(
        app.state
            .workspace_service
            .list_versions(&p.access, session_id)
            .await
            .unwrap()
            .len(),
        2
    );

    let (ambiguous_session, ambiguous_base, ambiguous_input) =
        example_workspace_with_rss(&app, b"step = 60;\nstep = 60;\n").await;
    let ambiguous = ModelPatch {
        table_kind: "rss_parameter".into(),
        row_key: BTreeMap::from([("name".into(), json!("step"))]),
        changes: BTreeMap::from([(
            "value".into(),
            ModelChange {
                from: json!(60),
                to: json!(900),
                unit: "s".into(),
            },
        )]),
    };
    let error = app
        .state
        .model_patch_service
        .modify(
            &p,
            ambiguous_session,
            ambiguous_input,
            ambiguous,
            Some(ambiguous_base),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code(), "row_not_unique");
    assert_eq!(
        app.state
            .workspace_service
            .list_versions(&p.access, ambiguous_session)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn checked_activation_and_rollback_only_move_workspace_pointers() {
    let app = TestApp::spawn().await;
    let p = principal();
    let (session_id, base_id) = workspace_with_csv(&app, &[("bus.csv", b"id,p\nb1,100\n")]).await;
    let input_id = app
        .state
        .workspace_service
        .inputs(&p.access, base_id)
        .await
        .unwrap()[0]
        .0
        .id;
    let child = app
        .state
        .model_patch_service
        .confirm_mapping(&p, session_id, input_id, "bus".into(), Some(base_id))
        .await
        .unwrap();
    let task = app
        .state
        .task_repo
        .create(
            &p.access,
            NewSimulationTask {
                session_id,
                status: "succeeded".into(),
                stage: Some("collecting".into()),
                study_type: "power_flow".into(),
                model_version_id: base_id,
                study_spec_json: json!({"kind":"power_flow"}).to_string(),
                script_artifact_id: app
                    .state
                    .workspace_service
                    .inputs(&p.access, base_id)
                    .await
                    .unwrap()[0]
                    .0
                    .artifact_id,
                agent_bundle_version: "v2.0.0".into(),
                agent_skill_refs_json: "[]".into(),
                engine_contract_version: "test-contract".into(),
                exit_code: Some(0),
                error_json: None,
                created_at_ms: 10,
                started_at_ms: Some(11),
                finished_at_ms: Some(12),
            },
        )
        .await
        .unwrap();
    let workspace = app
        .state
        .workspace_service
        .get(&p.access, session_id)
        .await
        .unwrap();
    let stale = app
        .state
        .workspace_service
        .activate_checked(&p.access, workspace.workspace.id, child.id, Some(base_id))
        .await
        .unwrap_err();
    assert!(matches!(
        stale,
        backend::application::WorkspaceError::VersionConflict
    ));
    app.state
        .workspace_service
        .activate_checked(&p.access, workspace.workspace.id, child.id, Some(child.id))
        .await
        .unwrap();
    app.state
        .workspace_service
        .rollback_checked(&p.access, workspace.workspace.id, base_id, Some(child.id))
        .await
        .unwrap();
    let unchanged = app
        .state
        .task_repo
        .find(&p.access, task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.status, "succeeded");
    assert_eq!(unchanged.model_version_id, task.model_version_id);
    assert_eq!(unchanged.study_spec_json, task.study_spec_json);
}
