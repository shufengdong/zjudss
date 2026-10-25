mod common;

use std::fmt::Write as _;
use std::io::Cursor;

use common::TestApp;
use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};
use zjudss::application::study_planning::{
    ResolvedSolver, SolverPreference, StudyPlanningError, StudyRequest, StudyStatus,
};
use zjudss::application::{PublishDraftRequest, UploadBatch, UploadFile};
use zjudss::auth::CurrentUser;
use zjudss::db::models::DraftRef;
use zjudss::storage::StageLimits;

type TableFixture<'a> = (&'a str, &'a [u8]);
type StudyCase<'a> = (&'a str, Vec<TableFixture<'a>>);

async fn apply_csv(
    app: &TestApp,
    user: &CurrentUser,
    session_id: i32,
    table_name: &str,
    csv: &[u8],
    expected: Option<DraftRef>,
    sequence: usize,
) -> DraftRef {
    let client_id = format!("study-{session_id}-{table_name}-{sequence}");
    let staged = app
        .state
        .artifact_service
        .store()
        .stage(Cursor::new(csv), StageLimits::default())
        .unwrap();
    let accepted = app
        .state
        .model_import_service
        .accept(
            user,
            session_id,
            client_id.clone(),
            expected,
            UploadBatch {
                client_batch_id: client_id.clone(),
                files: vec![UploadFile {
                    client_file_id: client_id,
                    file_name: format!("{table_name}.csv"),
                    relative_path: None,
                    media_type: Some("text/csv".into()),
                    staged,
                }],
            },
        )
        .await
        .unwrap();
    let import_id = accepted.id.parse().unwrap();
    let ready = app
        .state
        .model_import_service
        .process(user, import_id)
        .await
        .unwrap();
    let candidate = ready
        .candidates
        .into_iter()
        .find(|candidate| candidate.table_name.as_deref() == Some(table_name))
        .unwrap_or_else(|| panic!("{table_name} should be inferred from its fixture schema"));
    app.state
        .model_import_service
        .resolve_and_apply(user, import_id, expected, vec![candidate.id])
        .await
        .unwrap()
}

async fn publish_tables(
    app: &TestApp,
    user: &CurrentUser,
    title: &str,
    tables: &[(&str, &[u8])],
) -> i32 {
    let session = app
        .state
        .session_repo
        .create(&user.id, title, 1)
        .await
        .unwrap();
    let mut draft = None;
    for (sequence, (table_name, csv)) in tables.iter().enumerate() {
        draft = Some(apply_csv(app, user, session.id, table_name, csv, draft, sequence).await);
    }
    let draft = draft.unwrap();
    app.state
        .version_service
        .publish(
            user,
            session.id,
            PublishDraftRequest {
                expected_draft_id: draft.id,
                expected_draft_revision: draft.revision,
                client_publish_id: format!("publish-study-{}", session.id),
            },
        )
        .await
        .unwrap();
    session.id
}

fn power_flow() -> StudyRequest {
    StudyRequest::StaticPowerFlow {
        solver_preference: SolverPreference::Auto,
        requested_results: vec![],
    }
}

#[tokio::test]
async fn planning_requires_an_active_model_and_keeps_session_scope_private() {
    let app = TestApp::spawn().await;
    let alice = common::test_user("alice@example.com");
    let bob = common::test_user("bob@example.com");
    let session = app
        .state
        .session_repo
        .create(&alice.id, "empty", 1)
        .await
        .unwrap();

    let plan = app
        .state
        .study_planning_service
        .plan(&alice, session.id, power_flow())
        .await
        .unwrap();
    assert_eq!(plan.status, StudyStatus::MissingModelData);
    assert_eq!(plan.missing_parameters[0].field, "model.active");

    let foreign = app
        .state
        .study_planning_service
        .plan(&bob, session.id, power_flow())
        .await
        .unwrap_err();
    let absent = app
        .state
        .study_planning_service
        .plan(&bob, i32::MAX, power_flow())
        .await
        .unwrap_err();
    assert!(matches!(foreign, StudyPlanningError::NotFound));
    assert_eq!(foreign, absent);
    assert_eq!(foreign.code(), "not_found");
}

#[tokio::test]
async fn auto_power_flow_uses_radial_only_for_a_single_source_tree() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = publish_tables(
        &app,
        &user,
        "radial tree",
        &[
            ("bus", b"id\nsource\nb1\nb2\n"),
            ("line", b"name,bus_from,bus_to\nl1,source,b1\nl2,b1,b2\n"),
            ("voltage_source", b"name,bus\nsource,source\n"),
        ],
    )
    .await;

    let plan = app
        .state
        .study_planning_service
        .plan(&user, session_id, power_flow())
        .await
        .unwrap();
    assert_eq!(plan.status, StudyStatus::Ready);
    assert_eq!(plan.function.as_deref(), Some("ds_radial_pf"));
    assert_eq!(plan.solver, Some(ResolvedSolver::Radial));
    assert!(plan.model_facts.is_connected);
    assert!(!plan.model_facts.has_loops);
    assert!(!plan.model_facts.has_parallel_branches);
}

#[tokio::test]
async fn auto_power_flow_selects_newton_for_each_non_radial_model_fact() {
    let cases: Vec<StudyCase<'_>> = vec![
        (
            "loop",
            vec![
                ("bus", b"id\nsource\nb1\nb2\n"),
                (
                    "line",
                    b"name,bus_from,bus_to\nl1,source,b1\nl2,b1,b2\nl3,b2,source\n",
                ),
                ("voltage_source", b"name,bus\nsource,source\n"),
            ],
        ),
        (
            "parallel",
            vec![
                ("bus", b"id\nsource\nb1\n"),
                (
                    "line",
                    b"name,bus_from,bus_to\nl1,source,b1\nl2,source,b1\n",
                ),
                ("voltage_source", b"name,bus\nsource,source\n"),
            ],
        ),
        (
            "multiple sources",
            vec![
                ("bus", b"id\nsource\nb1\n"),
                ("line", b"name,bus_from,bus_to\nl1,source,b1\n"),
                (
                    "voltage_source",
                    b"name,bus\nsource-a,source\nsource-b,b1\n",
                ),
            ],
        ),
        (
            "pv generator",
            vec![
                ("bus", b"id\nsource\nb1\n"),
                ("line", b"name,bus_from,bus_to\nl1,source,b1\n"),
                ("voltage_source", b"name,bus\nsource,source\n"),
                ("generator", b"name,bus,execution_mode\ng1,b1,CONSTANT_PV\n"),
            ],
        ),
        (
            "grid forming inverter",
            vec![
                ("bus", b"id\nsource\nb1\n"),
                ("line", b"name,bus_from,bus_to\nl1,source,b1\n"),
                ("voltage_source", b"name,bus\nsource,source\n"),
                ("ibr", b"name,bus,port_model\ni1,b1,GFM\n"),
            ],
        ),
    ];

    for (title, tables) in cases {
        let app = TestApp::spawn().await;
        let user = common::test_user("alice@example.com");
        let session_id = publish_tables(&app, &user, title, &tables).await;
        let plan = app
            .state
            .study_planning_service
            .plan(&user, session_id, power_flow())
            .await
            .unwrap();
        assert_eq!(plan.status, StudyStatus::Ready, "{title}");
        assert_eq!(
            plan.function.as_deref(),
            Some("ds_newton_raphson_pf"),
            "{title}"
        );
        assert_eq!(plan.solver, Some(ResolvedSolver::Newton), "{title}");
    }
}

#[tokio::test]
async fn topology_respects_open_edges_and_connects_multi_winding_transformers() {
    let open_app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let open_session = publish_tables(
        &open_app,
        &user,
        "open edge",
        &[
            ("bus", b"id\nsource\nb1\nb2\n"),
            (
                "line",
                b"name,bus_from,bus_to,open_to\nl1,source,b1,false\nl2,b1,b2,true\n",
            ),
            ("voltage_source", b"name,bus\nsource,source\n"),
        ],
    )
    .await;
    let open_plan = open_app
        .state
        .study_planning_service
        .plan(&user, open_session, power_flow())
        .await
        .unwrap();
    assert!(!open_plan.model_facts.is_connected);
    assert_eq!(open_plan.solver, Some(ResolvedSolver::Newton));
    assert_eq!(open_plan.function.as_deref(), Some("ds_newton_raphson_pf"));

    let winding_app = TestApp::spawn().await;
    let winding_session = publish_tables(
        &winding_app,
        &user,
        "multi winding",
        &[
            ("bus", b"id\nsource\nb1\nb2\n"),
            ("voltage_source", b"name,bus\nsource,source\n"),
            (
                "transformer_winding",
                b"transformer,bus,winding\nt1,source,1\nt1,b1,2\nt1,b2,3\n",
            ),
        ],
    )
    .await;
    let winding_plan = winding_app
        .state
        .study_planning_service
        .plan(&user, winding_session, power_flow())
        .await
        .unwrap();
    assert!(winding_plan.model_facts.is_connected);
    assert!(!winding_plan.model_facts.has_loops);
    assert_eq!(winding_plan.solver, Some(ResolvedSolver::Radial));
}

#[tokio::test]
async fn topology_scans_beyond_the_public_five_hundred_row_page_limit() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let mut buses = String::from("id\nsource\n");
    let mut lines = String::from("name,bus_from,bus_to\n");
    for index in 0..601 {
        writeln!(buses, "b{index}").unwrap();
        let from = if index == 0 {
            "source".to_string()
        } else {
            format!("b{}", index - 1)
        };
        writeln!(lines, "l{index},{from},b{index}").unwrap();
    }
    let session_id = publish_tables(
        &app,
        &user,
        "large radial model",
        &[
            ("bus", buses.as_bytes()),
            ("line", lines.as_bytes()),
            ("voltage_source", b"name,bus\nsource,source\n"),
        ],
    )
    .await;

    let plan = app
        .state
        .study_planning_service
        .plan(&user, session_id, power_flow())
        .await
        .unwrap();
    assert_eq!(plan.model_facts.bus_count, 602);
    assert!(plan.model_facts.is_connected);
    assert_eq!(plan.solver, Some(ResolvedSolver::Radial));
}

#[tokio::test]
async fn model_summary_counts_devices_spectra_and_supported_fault_equivalents() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session_id = publish_tables(
        &app,
        &user,
        "study capabilities",
        &[
            ("bus", b"id\nsource\nb1\n"),
            ("line", b"name,bus_from,bus_to\nl1,source,b1\n"),
            (
                "voltage_source",
                b"name,bus,sc_va,x_r\nsource,source,1000000,10\n",
            ),
            (
                "generator",
                b"name,bus,execution_mode,z_r,z_x,spectrum\ng1,b1,CONSTANT_PQ,0.1,0.2,harmonics\n",
            ),
            ("ibr", b"name,bus,port_model\ni1,b1,GFL\n"),
            ("motor", b"name,bus,motor_marker\nm1,b1,motor\n"),
            ("storage", b"name,bus,storage_marker\nbattery,b1,storage\n"),
            ("spectrum", b"name,harmonic\nharmonics,1\n"),
        ],
    )
    .await;

    let plan = app
        .state
        .study_planning_service
        .plan(&user, session_id, power_flow())
        .await
        .unwrap();
    assert_eq!(plan.model_facts.voltage_source_count, 1);
    assert_eq!(plan.model_facts.generator_count, 1);
    assert_eq!(plan.model_facts.ibr_count, 1);
    assert_eq!(plan.model_facts.motor_count, 1);
    assert_eq!(plan.model_facts.storage_count, 1);
    assert_eq!(plan.model_facts.spectrum_count, 1);
    assert_eq!(plan.model_facts.referenced_spectrum_count, 1);
    assert_eq!(plan.model_facts.fault_source_equivalent_count, 2);
}

#[tokio::test]
async fn schema_drift_and_unreadable_owned_parquet_return_a_stable_error() {
    let user = common::test_user("alice@example.com");

    let schema_app = TestApp::spawn().await;
    let schema_session = publish_tables(
        &schema_app,
        &user,
        "schema drift",
        &[
            ("bus", b"id\nsource\n"),
            ("voltage_source", b"name,bus\nsource,source\n"),
        ],
    )
    .await;
    let schema_workspace = schema_app
        .state
        .version_service
        .workspace(&user, schema_session)
        .await
        .unwrap();
    let active = schema_workspace.active.as_ref().unwrap();
    {
        let mut connection = schema_app.pool.get().unwrap();
        diesel::sql_query(
            "INSERT INTO model_versions(workspace_id,content_hash,source_summary_json,engine_contract_version,created_at_ms) \
             SELECT workspace_id,?1,source_summary_json,engine_contract_version,created_at_ms+1 FROM model_versions WHERE id=?2",
        )
            .bind::<Text, _>("dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd")
            .bind::<Integer, _>(active.version.id)
            .execute(&mut connection)
            .unwrap();
        let drifted_version_id: i32 =
            diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()"))
                .get_result(&mut connection)
                .unwrap();
        diesel::sql_query(
            "INSERT INTO model_dataframes(version_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) \
             SELECT ?1,table_name,artifact_id,CASE WHEN table_name='bus' THEN ?2 ELSE schema_json END,row_count,content_hash,created_at_ms \
             FROM model_dataframes WHERE version_id=?3",
        )
        .bind::<Integer, _>(drifted_version_id)
        .bind::<Text, _>("[")
        .bind::<Integer, _>(active.version.id)
        .execute(&mut connection)
        .unwrap();
        diesel::sql_query("UPDATE workspaces SET active_version_id=?1 WHERE id=?2")
            .bind::<Integer, _>(drifted_version_id)
            .bind::<Integer, _>(active.version.workspace_id)
            .execute(&mut connection)
            .unwrap();
    }
    let schema_error = schema_app
        .state
        .study_planning_service
        .plan(&user, schema_session, power_flow())
        .await
        .unwrap_err();
    assert!(matches!(
        schema_error,
        StudyPlanningError::ModelReadFailed(_)
    ));
    assert_eq!(schema_error.code(), "study_model_read_failed");

    let parquet_app = TestApp::spawn().await;
    let parquet_session = publish_tables(
        &parquet_app,
        &user,
        "unreadable parquet",
        &[
            ("bus", b"id\nsource\n"),
            ("voltage_source", b"name,bus\nsource,source\n"),
        ],
    )
    .await;
    let parquet_workspace = parquet_app
        .state
        .version_service
        .workspace(&user, parquet_session)
        .await
        .unwrap();
    let bus_artifact_id = parquet_workspace
        .active
        .as_ref()
        .unwrap()
        .dataframes
        .iter()
        .find(|table| table.table_name == "bus")
        .unwrap()
        .artifact_id;
    let scope = parquet_app
        .state
        .artifact_service
        .resolve_scope(user.storage_scope_id)
        .await
        .unwrap();
    let artifact = parquet_app
        .state
        .artifact_service
        .find_owned(scope.id, bus_artifact_id)
        .await
        .unwrap();
    let path = parquet_app
        .state
        .artifact_service
        .store()
        .resolve(&artifact.storage_path)
        .unwrap();
    std::fs::write(path, b"not parquet").unwrap();
    let parquet_error = parquet_app
        .state
        .study_planning_service
        .plan(&user, parquet_session, power_flow())
        .await
        .unwrap_err();
    assert!(matches!(
        parquet_error,
        StudyPlanningError::ModelReadFailed(_)
    ));
    assert_eq!(parquet_error.code(), "study_model_read_failed");
}
