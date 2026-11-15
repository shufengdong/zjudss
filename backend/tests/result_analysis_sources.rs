mod common;

use common::TestApp;
use diesel::prelude::*;
use zjudss::application::{
    AnalysisInput, AnalysisInputSource, AnalysisRequest, PublishDraftRequest,
};
use zjudss::auth::CurrentUser;
use zjudss::db::models::ResultAnalysisInput;
use zjudss::db::repositories::{
    AnalysisRepo, DatasetModelOrigin, RepoError, ResultRepo, VersionRepo, VersionRepoError,
    VersionSnapshot,
};

async fn publish_version(
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

fn insert_raw_dataset(
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

fn insert_derived_dataset(app: &TestApp, session_id: i32, artifact_id: i32, suffix: &str) -> i32 {
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("INSERT INTO result_analysis_runs(session_id,intent_json,script_artifact_id,status,stage,agent_bundle_version,agent_skill_refs_json,engine_contract_version,engine_resource_snapshot_id,created_at_ms) VALUES(?1,'{}',?2,'succeeded','present','test','[]','test',(SELECT id FROM engine_resource_snapshots LIMIT 1),4)")
        .bind::<diesel::sql_types::Integer, _>(session_id)
        .bind::<diesel::sql_types::Integer, _>(artifact_id)
        .execute(&mut conn)
        .unwrap();
    let analysis_id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result::<i32>(&mut conn)
    .unwrap();
    diesel::sql_query("INSERT INTO result_datasets(analysis_run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms) VALUES(?1,?2,?3,'[]','hash',1,1,5)")
        .bind::<diesel::sql_types::Integer, _>(analysis_id)
        .bind::<diesel::sql_types::Integer, _>(artifact_id)
        .bind::<diesel::sql_types::Text, _>(format!("derived-{suffix}"))
        .execute(&mut conn)
        .unwrap();
    diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
        "last_insert_rowid()",
    ))
    .get_result(&mut conn)
    .unwrap()
}

fn link_dataset(app: &TestApp, derived: i32, source: i32) {
    diesel::sql_query("INSERT INTO dataset_lineage_snapshots(derived_dataset_id,source_kind,source_dataset_id,source_name,source_schema_json,source_schema_hash,source_content_hash,source_producer_json,created_at_ms) SELECT ?1,'dataset',id,result_name,schema_json,schema_hash,schema_hash,'{}',6 FROM result_datasets WHERE id=?2")
        .bind::<diesel::sql_types::Integer, _>(derived)
        .bind::<diesel::sql_types::Integer, _>(source)
        .execute(&mut app.pool.get().unwrap())
        .unwrap();
}

#[tokio::test]
async fn dataset_model_origin_handles_direct_none_multiple_cycle_and_depth_limit() {
    let app = TestApp::spawn().await;
    let user = common::test_user("alice@example.com");
    let session = app
        .state
        .session_repo
        .create(&user.id, "origin", 1)
        .await
        .unwrap();
    let first = publish_version(&app, &user, session.id, "first", b"id,name\n1,first\n").await;
    let direct_first = insert_raw_dataset(&app, session.id, &first, "first");
    let second = publish_version(&app, &user, session.id, "second", b"id,name\n1,second\n").await;
    let direct_second = insert_raw_dataset(&app, session.id, &second, "second");
    let artifact_id = second.dataframes[0].artifact_id;
    let repo = ResultRepo::new(app.pool.clone());

    assert_eq!(
        repo.dataset_model_origin(&user.id, session.id, direct_second, 64)
            .await
            .unwrap(),
        DatasetModelOrigin::Unique(second.version.id)
    );

    let model_analysis = app
        .state
        .analysis_service
        .run(AnalysisRequest {
            principal: user.clone(),
            session_id: session.id,
            request_message_id: None,
            intent: serde_json::json!("active model analysis"),
            script: "result = limit(1, model);".into(),
            inputs: vec![AnalysisInput {
                source: AnalysisInputSource::ActiveModelDataFrameId(second.dataframes[0].id),
                alias: "model".into(),
            }],
            expected_outputs: vec!["result".into()],
            skill_refs: vec![],
        })
        .await
        .unwrap();
    assert!(!model_analysis.run.engine_resource_snapshot_id.is_empty());
    let model_lineage = repo
        .lineage(&user.id, model_analysis.datasets[0].id)
        .await
        .unwrap();
    assert_eq!(model_lineage.len(), 1);
    assert_eq!(model_lineage[0].source_kind, "active_model");
    assert_eq!(model_lineage[0].model_version_id, Some(second.version.id));
    assert_eq!(
        model_lineage[0].model_dataframe_id,
        Some(second.dataframes[0].id)
    );
    assert!(!model_lineage[0].deleted);

    let none = insert_derived_dataset(&app, session.id, artifact_id, "none");
    assert_eq!(
        repo.dataset_model_origin(&user.id, session.id, none, 64)
            .await
            .unwrap(),
        DatasetModelOrigin::None
    );

    let multiple = insert_derived_dataset(&app, session.id, artifact_id, "multiple");
    link_dataset(&app, multiple, direct_first);
    link_dataset(&app, multiple, direct_second);
    assert_eq!(
        repo.dataset_model_origin(&user.id, session.id, multiple, 64)
            .await
            .unwrap(),
        DatasetModelOrigin::Multiple(vec![first.version.id, second.version.id])
    );

    let cycle_a = insert_derived_dataset(&app, session.id, artifact_id, "cycle-a");
    let cycle_b = insert_derived_dataset(&app, session.id, artifact_id, "cycle-b");
    link_dataset(&app, cycle_a, cycle_b);
    link_dataset(&app, cycle_b, cycle_a);
    assert_eq!(
        repo.dataset_model_origin(&user.id, session.id, cycle_a, 64)
            .await
            .unwrap(),
        DatasetModelOrigin::Invalid
    );

    let mut source = direct_second;
    for depth in 0..64 {
        let derived =
            insert_derived_dataset(&app, session.id, artifact_id, &format!("depth-{depth}"));
        link_dataset(&app, derived, source);
        source = derived;
    }
    assert_eq!(
        repo.dataset_model_origin(&user.id, session.id, source, 64)
            .await
            .unwrap(),
        DatasetModelOrigin::Unique(second.version.id)
    );
    let too_deep = insert_derived_dataset(&app, session.id, artifact_id, "depth-64");
    link_dataset(&app, too_deep, source);
    source = too_deep;
    assert_eq!(
        repo.dataset_model_origin(&user.id, session.id, source, 64)
            .await
            .unwrap(),
        DatasetModelOrigin::Invalid
    );

    for dataset_id in [direct_first, none, multiple, cycle_a, source] {
        let error = app
            .state
            .analysis_service
            .run(AnalysisRequest {
                principal: user.clone(),
                session_id: session.id,
                request_message_id: None,
                intent: serde_json::json!("mixed origin"),
                script: "invalid before inspector".into(),
                inputs: vec![
                    AnalysisInput {
                        source: AnalysisInputSource::ActiveModelDataFrameId(
                            second.dataframes[0].id,
                        ),
                        alias: "model".into(),
                    },
                    AnalysisInput {
                        source: AnalysisInputSource::DatasetId(dataset_id),
                        alias: "result".into(),
                    },
                ],
                expected_outputs: vec!["output".into()],
                skill_refs: vec![],
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "analysis_model_origin_mismatch");
        assert_eq!(error.run_id, None);
    }
}

#[tokio::test]
async fn active_dataframe_lookup_is_exactly_scoped_and_create_rejects_switch_race() {
    let app = TestApp::spawn().await;
    let alice = common::test_user("alice@example.com");
    let bob = common::test_user("bob@example.com");
    let alice_session = app
        .state
        .session_repo
        .create(&alice.id, "alice", 1)
        .await
        .unwrap();
    let bob_session = app
        .state
        .session_repo
        .create(&bob.id, "bob", 1)
        .await
        .unwrap();
    let other_alice_session = app
        .state
        .session_repo
        .create(&alice.id, "other alice", 1)
        .await
        .unwrap();
    let historical = publish_version(
        &app,
        &alice,
        alice_session.id,
        "historical",
        b"id,name\n1,old\n",
    )
    .await;
    let active = publish_version(
        &app,
        &alice,
        alice_session.id,
        "active",
        b"id,name\n1,new\n",
    )
    .await;
    let foreign = publish_version(&app, &bob, bob_session.id, "foreign", b"id,name\n1,bob\n").await;
    let other_session = publish_version(
        &app,
        &alice,
        other_alice_session.id,
        "other-session",
        b"id,name\n1,other\n",
    )
    .await;
    let repo = VersionRepo::new(app.pool.clone());
    let active_frame = active.dataframes[0].clone();

    let found = repo
        .find_active_version_dataframe(&alice.id, alice_session.id, active_frame.id)
        .await
        .unwrap();
    assert_eq!(
        (found.id, found.version_id),
        (active_frame.id, active.version.id)
    );
    for dataframe_id in [
        historical.dataframes[0].id,
        foreign.dataframes[0].id,
        other_session.dataframes[0].id,
    ] {
        assert!(matches!(
            repo.find_active_version_dataframe(&alice.id, alice_session.id, dataframe_id)
                .await,
            Err(VersionRepoError::Repository(RepoError::NotFound))
        ));
    }

    let draft = common::publish_model_bytes(
        &app,
        &alice,
        alice_session.id,
        "draft-only",
        "bus.csv",
        b"id,name\n1,draft\n",
        None,
        Some("bus"),
    )
    .await;
    let draft_frame_id = zjudss::db::schema::draft_dataframes::table
        .filter(zjudss::db::schema::draft_dataframes::draft_id.eq(draft.id))
        .select(zjudss::db::schema::draft_dataframes::id)
        .first::<i32>(&mut app.pool.get().unwrap())
        .unwrap();
    assert!(matches!(
        repo.find_active_version_dataframe(&alice.id, alice_session.id, draft_frame_id)
            .await,
        Err(VersionRepoError::Repository(RepoError::NotFound))
    ));

    diesel::sql_query("UPDATE workspaces SET active_version_id=?1 WHERE session_id=?2")
        .bind::<diesel::sql_types::Integer, _>(historical.version.id)
        .bind::<diesel::sql_types::Integer, _>(alice_session.id)
        .execute(&mut app.pool.get().unwrap())
        .unwrap();
    let input = ResultAnalysisInput {
        analysis_run_id: 0,
        source_kind: "active_model".into(),
        dataset_id: None,
        resource_id: None,
        model_version_id: Some(active.version.id),
        model_dataframe_id: Some(active_frame.id),
        alias: "bus_model".into(),
        ordinal: 0,
        source_name: active_frame.table_name,
        source_schema_json: active_frame.schema_json,
        source_schema_hash: "schema".into(),
        source_content_hash: active_frame.content_hash,
        source_producer_json: "{}".into(),
    };
    let create = AnalysisRepo::new(app.pool.clone())
        .create(
            &alice.id,
            zjudss::db::models::NewResultAnalysisRun {
                session_id: alice_session.id,
                request_message_id: None,
                intent_json: "{}".into(),
                script_artifact_id: active_frame.artifact_id,
                status: "pending".into(),
                stage: "script".into(),
                error_json: None,
                agent_bundle_version: "test".into(),
                agent_skill_refs_json: "[]".into(),
                engine_contract_version: "test".into(),
                engine_resource_snapshot_id: "test".into(),
                created_at_ms: 1,
                started_at_ms: None,
                finished_at_ms: None,
            },
            vec![input],
        )
        .await;
    assert!(matches!(create, Err(RepoError::VersionConflict)));

    diesel::sql_query("UPDATE model_versions SET deleted_at_ms=2,tombstone_json='{}' WHERE id=?1")
        .bind::<diesel::sql_types::Integer, _>(historical.version.id)
        .execute(&mut app.pool.get().unwrap())
        .unwrap();
    assert!(matches!(
        repo.find_active_version_dataframe(
            &alice.id,
            alice_session.id,
            historical.dataframes[0].id,
        )
        .await,
        Err(VersionRepoError::Repository(RepoError::NotFound))
    ));
}
