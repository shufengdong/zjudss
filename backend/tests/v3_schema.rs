use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use tempfile::TempDir;

fn database() -> (TempDir, zjudss::db::DbPool) {
    let root = tempfile::tempdir().expect("temp database directory");
    let url = format!("sqlite://{}", root.path().join("v3.db").display());
    let pool = zjudss::db::establish_pool(&url).expect("create pool");
    zjudss::db::run_migrations(&mut pool.get().expect("checkout")).expect("run initial migration");
    (root, pool)
}

fn base_rows(conn: &mut SqliteConnection) {
    conn.batch_execute(
        r#"
        INSERT INTO storage_scopes(id,kind,scope_key,created_at_ms) VALUES(1,'user','u1',1);
        INSERT INTO users(id,name,password_hash,storage_scope_id,created_at_ms)
          VALUES('u1','User','$argon2id$fixture',1,1);
        INSERT INTO sessions(id,title,created_at_ms,updated_at_ms,user_id,title_source)
          VALUES(1,'one',1,1,'u1','default'),
                (2,'two',1,1,'u1','default');
        INSERT INTO workspaces(id,session_id,created_at_ms,updated_at_ms)
          VALUES(1,1,1,1),(2,2,1,1);
        INSERT INTO artifacts(id,sha256,size_bytes,storage_path,format,media_type,created_at_ms)
          VALUES(1,printf('%064d',1),10,'a/1','parquet','application/vnd.apache.parquet',1),
                (2,printf('%064d',2),10,'a/2','rss','text/plain',1),
                (3,printf('%064d',3),10,'a/3','parquet','application/vnd.apache.parquet',1);
        "#,
    )
    .expect("seed base rows");
}

#[test]
fn v3_replaces_v2_lifecycle_tables() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    #[derive(QueryableByName)]
    struct Name {
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
    }
    let names =
        diesel::sql_query("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .load::<Name>(&mut conn)
            .unwrap()
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>();
    for required in [
        "workspace_drafts",
        "draft_dataframes",
        "model_imports",
        "model_import_candidates",
        "model_import_selections",
        "model_dataframes",
        "publish_receipts",
        "script_resources",
        "simulation_runs",
        "dataset_lineage_snapshots",
        "version_deletion_previews",
    ] {
        assert!(names.iter().any(|name| name == required), "{required}");
    }
    for removed in [
        "ingestions",
        "ingestion_artifacts",
        "version_artifacts",
        "dataframe_bindings",
        "simulation_tasks",
    ] {
        assert!(!names.iter().any(|name| name == removed), "{removed}");
    }
}

#[test]
fn draft_and_published_dataframe_constraints_are_database_enforced() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        r#"
        INSERT INTO workspace_drafts(id,workspace_id,revision,diagnostics_json,created_at_ms,updated_at_ms)
          VALUES(10,1,1,'{}',1,1);
        INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms)
          VALUES(10,'bus',1,'{}',1,printf('%064d',11),1);
        INSERT INTO model_versions(id,workspace_id,content_hash,source_summary_json,engine_contract_version,created_at_ms)
          VALUES(20,1,printf('%064d',20),'{}','engine-v3',1);
        INSERT INTO model_dataframes(version_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms)
          VALUES(20,'bus',1,'{}',1,printf('%064d',11),1);
        UPDATE workspaces SET active_version_id=20 WHERE id=1;
        "#,
    )
    .unwrap();

    assert!(conn.batch_execute(
        "INSERT INTO workspace_drafts(workspace_id,revision,diagnostics_json,created_at_ms,updated_at_ms)
         VALUES(1,1,'{}',1,1)"
    ).is_err());
    assert!(conn.batch_execute(
        "INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms)
         VALUES(10,'bus',3,'{}',1,printf('%064d',12),1)"
    ).is_err());
    assert!(
        conn.batch_execute(
            "UPDATE model_dataframes SET table_name='line' WHERE version_id=20 AND table_name='bus'"
        )
        .is_err()
    );
    assert!(
        conn.batch_execute("UPDATE workspaces SET active_version_id=20 WHERE id=2")
            .is_err()
    );
}

#[test]
fn runs_bind_same_session_model_and_script_and_keep_idempotency() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        r#"
        INSERT INTO model_versions(id,workspace_id,content_hash,source_summary_json,engine_contract_version,created_at_ms)
          VALUES(20,1,printf('%064d',20),'{}','engine-v3',1);
        UPDATE workspaces SET active_version_id=20 WHERE id=1;
        INSERT INTO script_resources(id,session_id,artifact_id,name,source_kind,content_hash,
          validation_status,required_aliases_json,outputs_json,diagnostics_json,
          agent_bundle_version,agent_skill_refs_json,created_at_ms)
          VALUES(30,1,2,'run.rss','upload',printf('%064d',2),'valid','[]','["out"]','{}',NULL,'[]',1);
        INSERT INTO simulation_runs(id,session_id,model_version_id,model_content_hash,
          script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,
          engine_contract_version,agent_bundle_version,agent_skill_refs_json,created_at_ms)
          VALUES(40,1,20,printf('%064d',20),30,2,printf('%064d',2),'click-1','queued','queued',
          'engine-v3',NULL,'[]',1);
        "#,
    )
    .unwrap();
    assert!(
        conn.batch_execute(
            "INSERT INTO simulation_runs(session_id,model_version_id,model_content_hash,
         script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,
         engine_contract_version,agent_skill_refs_json,created_at_ms)
         VALUES(1,20,printf('%064d',20),30,2,printf('%064d',2),'click-1','queued','queued',
         'engine-v3','[]',1)"
        )
        .is_err()
    );
    assert!(
        conn.batch_execute(
            "INSERT INTO simulation_runs(session_id,model_version_id,model_content_hash,
         script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,
         engine_contract_version,agent_skill_refs_json,created_at_ms)
         VALUES(2,20,printf('%064d',20),30,2,printf('%064d',2),'click-2','queued','queued',
         'engine-v3','[]',1)"
        )
        .is_err()
    );
}

#[test]
fn deleted_datasets_drop_artifacts_but_lineage_snapshots_survive() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        r#"
        INSERT INTO model_versions(id,workspace_id,content_hash,source_summary_json,engine_contract_version,created_at_ms)
          VALUES(20,1,printf('%064d',20),'{}','engine-v3',1);
        INSERT INTO script_resources(id,session_id,artifact_id,name,source_kind,content_hash,
          validation_status,required_aliases_json,outputs_json,diagnostics_json,
          agent_bundle_version,agent_skill_refs_json,created_at_ms)
          VALUES(30,1,2,'run.rss','upload',printf('%064d',2),'valid','[]','["out"]','{}',NULL,'[]',1);
        INSERT INTO simulation_runs(id,session_id,model_version_id,model_content_hash,
          script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,
          engine_contract_version,agent_bundle_version,agent_skill_refs_json,created_at_ms)
          VALUES(40,1,20,printf('%064d',20),30,2,printf('%064d',2),'click-1','succeeded','complete',
          'engine-v3',NULL,'[]',1);
        INSERT INTO result_datasets(id,run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms)
          VALUES(50,40,1,'raw','{}',printf('%064d',50),1,10,1);
        INSERT INTO result_analysis_runs(id,session_id,intent_json,script_artifact_id,status,stage,
          agent_bundle_version,agent_skill_refs_json,engine_contract_version,created_at_ms)
          VALUES(60,1,'{}',2,'succeeded','complete','v3','[]','engine-v3',1);
        INSERT INTO result_datasets(id,analysis_run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms)
          VALUES(51,60,3,'derived','{}',printf('%064d',51),1,10,1);
        INSERT INTO dataset_lineage_snapshots(derived_dataset_id,source_dataset_id,source_name,
          source_schema_json,source_schema_hash,source_producer_json,created_at_ms)
          VALUES(51,50,'raw','{}',printf('%064d',50),'{"runId":"40"}',1);
        UPDATE result_datasets
          SET artifact_id=NULL,deleted_at_ms=2,tombstone_json='{"name":"raw"}' WHERE id=50;
        "#,
    )
    .unwrap();
    assert!(conn.batch_execute(
        "INSERT INTO result_datasets(run_id,artifact_id,result_name,schema_json,schema_hash,row_count,
         size_bytes,created_at_ms,deleted_at_ms,tombstone_json)
         VALUES(40,1,'invalid','{}',printf('%064d',52),1,10,1,2,'{}')"
    ).is_err());

    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let count = diesel::sql_query(
        "SELECT COUNT(*) AS count FROM dataset_lineage_snapshots WHERE source_dataset_id=50",
    )
    .get_result::<Count>(&mut conn)
    .unwrap()
    .count;
    assert_eq!(count, 1);
}
