use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use tempfile::TempDir;

fn database() -> (TempDir, zjudss::db::DbPool) {
    let root = tempfile::tempdir().expect("temp database directory");
    let url = format!("sqlite://{}", root.path().join("v4.db").display());
    let pool = zjudss::db::establish_pool(&url).expect("create pool");
    zjudss::db::run_migrations(&mut pool.get().expect("checkout")).expect("run migration");
    (root, pool)
}

fn base_rows(conn: &mut SqliteConnection) {
    conn.batch_execute(
        r#"
        INSERT INTO storage_scopes(id,kind,scope_key,created_at_ms) VALUES(1,'user','u1',1);
        INSERT INTO users(id,name,password_hash,storage_scope_id,created_at_ms)
          VALUES('u1','User','$argon2id$fixture',1,1);
        INSERT INTO sessions(id,title,created_at_ms,updated_at_ms,user_id,title_source)
          VALUES(1,'one',1,1,'u1','default'),(2,'two',1,1,'u1','default');
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

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

#[test]
fn v4_initial_schema_contains_only_current_tables() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    let names = diesel::sql_query("SELECT name FROM sqlite_master WHERE type='table'")
        .load::<NameRow>(&mut conn)
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();
    for required in [
        "engine_resource_snapshots",
        "upload_batches",
        "upload_files",
        "upload_candidates",
        "data_resources",
        "data_resource_deletion_previews",
        "data_resource_replacement_previews",
        "session_deletion_previews",
        "run_deletion_previews",
        "study_configurations",
        "study_input_bindings",
        "study_struct_values",
        "study_result_selections",
        "managed_workflows",
        "workflow_input_snapshots",
        "workflow_attempts",
        "retry_previews",
    ] {
        assert!(
            names.iter().any(|name| name == required),
            "missing {required}"
        );
    }
    for removed in [
        "model_imports",
        "model_import_candidates",
        "model_import_selections",
    ] {
        assert!(
            !names.iter().any(|name| name == removed),
            "obsolete {removed}"
        );
    }
}

#[test]
fn engine_resource_snapshot_persists_case_projection_catalog_version() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    let columns = diesel::sql_query("PRAGMA table_info(engine_resource_snapshots)")
        .load::<NameRow>(&mut conn)
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();
    assert!(
        columns
            .iter()
            .any(|name| name == "case_projections_schema_version")
    );
}

#[test]
fn one_live_data_alias_and_one_nonterminal_workflow_are_database_constraints() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        "INSERT INTO data_resources(id,session_id,display_name,alias,normalized_alias,status,source_kind,normalized_artifact_id,schema_json,schema_hash,row_count,lineage_json,created_at_ms,updated_at_ms)
         VALUES('r1',1,'One','Load_Profile','load_profile','live','upload',1,'[]',printf('%064d',1),1,'{}',1,1);
         INSERT INTO managed_workflows(id,session_id,status,intent_json,created_at_ms,updated_at_ms)
         VALUES(1,1,'queued','{}',1,1);",
    ).unwrap();
    assert!(conn.batch_execute(
        "INSERT INTO data_resources(id,session_id,display_name,alias,normalized_alias,status,source_kind,normalized_artifact_id,schema_json,schema_hash,row_count,lineage_json,created_at_ms,updated_at_ms)
         VALUES('r2',1,'Two','load_profile','load_profile','live','upload',3,'[]',printf('%064d',2),1,'{}',1,1)"
    ).is_err());
    assert!(conn.batch_execute(
        "INSERT INTO managed_workflows(id,session_id,status,intent_json,created_at_ms,updated_at_ms)
         VALUES(2,1,'executing','{}',1,1)"
    ).is_err());
    conn.batch_execute("UPDATE data_resources SET status='deleted',normalized_artifact_id=NULL,source_artifact_id=NULL,deleted_at_ms=2 WHERE id='r1'; UPDATE managed_workflows SET status='failed',finished_at_ms=2 WHERE id=1;").unwrap();
    conn.batch_execute(
        "INSERT INTO data_resources(id,session_id,display_name,alias,normalized_alias,status,source_kind,normalized_artifact_id,schema_json,schema_hash,row_count,lineage_json,created_at_ms,updated_at_ms)
         VALUES('r2',1,'Two','load_profile','load_profile','live','upload',3,'[]',printf('%064d',2),1,'{}',2,2);
         INSERT INTO managed_workflows(id,session_id,status,intent_json,created_at_ms,updated_at_ms)
         VALUES(2,1,'executing','{}',2,2);"
    ).unwrap();
}

#[test]
fn para_resources_cannot_join_model_versions() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        "INSERT INTO model_versions(id,workspace_id,description,content_hash,source_summary_json,engine_contract_version,created_at_ms)
         VALUES(20,1,'first',printf('%064d',20),'{}','engine-v4',1);
         INSERT INTO data_resources(id,session_id,display_name,alias,normalized_alias,status,declared_schema_ref,source_kind,normalized_artifact_id,schema_json,schema_hash,row_count,lineage_json,created_at_ms,updated_at_ms)
         VALUES('para1',1,'Parameters','para','para','live','dss/para','static_import',1,'[]',printf('%064d',1),1,'{}',1,1);"
    ).unwrap();
    assert!(conn.batch_execute(
        "INSERT INTO model_dataframes(version_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms)
         SELECT 20,alias,normalized_artifact_id,schema_json,row_count,schema_hash,2 FROM data_resources WHERE id='para1'"
    ).is_err());
}

#[test]
fn model_version_description_is_frozen_and_nonempty() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    assert!(conn.batch_execute(
        "INSERT INTO model_versions(workspace_id,description,content_hash,source_summary_json,engine_contract_version,created_at_ms)
         VALUES(1,'',printf('%064d',20),'{}','engine-v4',1)"
    ).is_err());
    conn.batch_execute(
        "INSERT INTO model_versions(id,workspace_id,description,content_hash,source_summary_json,engine_contract_version,created_at_ms)
         VALUES(20,1,'首次发布：1 张模型表',printf('%064d',20),'{}','engine-v4',1)"
    ).unwrap();
    assert!(
        conn.batch_execute("UPDATE model_versions SET description='changed' WHERE id=20")
            .is_err()
    );
}

#[test]
fn resource_replacement_preserves_frozen_artifacts() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        "INSERT INTO data_resources(id,session_id,display_name,alias,normalized_alias,status,source_kind,normalized_artifact_id,schema_json,schema_hash,row_count,lineage_json,created_at_ms,updated_at_ms)
         VALUES('old',1,'Old','profile','profile','live','upload',1,'[]',printf('%064d',1),1,'{}',1,1),
               ('new',1,'New','data_new','data_new','live','upload',3,'[]',printf('%064d',3),1,'{}',1,1);
         UPDATE data_resources SET status='replaced',alias='data_old',normalized_alias='data_old',replaced_by_resource_id='new',updated_at_ms=2 WHERE id='old';
         UPDATE data_resources SET alias='profile',normalized_alias='profile',updated_at_ms=2 WHERE id='new';"
    ).unwrap();
    #[derive(QueryableByName)]
    struct ArtifactRow {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        normalized_artifact_id: i32,
    }
    let row = diesel::sql_query("SELECT normalized_artifact_id FROM data_resources WHERE id='old'")
        .get_result::<ArtifactRow>(&mut conn)
        .unwrap();
    assert_eq!(row.normalized_artifact_id, 1);
    assert!(
        conn.batch_execute("UPDATE data_resources SET normalized_artifact_id=3 WHERE id='old'")
            .is_err()
    );
}

#[test]
fn run_and_result_tombstones_are_consistent() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        "INSERT INTO script_resources(id,session_id,artifact_id,name,source_kind,content_hash,validation_status,required_aliases_json,outputs_json,diagnostics_json,agent_skill_refs_json,created_at_ms)
         VALUES(30,1,2,'run.rss','manual',printf('%064d',2),'valid','[]','[]','{}','[]',1);
         INSERT INTO simulation_runs(id,session_id,script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,created_at_ms)
         VALUES(40,1,30,2,printf('%064d',2),'run-1','succeeded','complete','engine-v4','[]',1);
         INSERT INTO result_datasets(id,run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms)
         VALUES(50,40,1,'out','[]',printf('%064d',1),1,10,1);
         UPDATE simulation_runs SET deleted_at_ms=2,tombstone_json='{}' WHERE id=40;"
    ).unwrap();
    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let count = diesel::sql_query("SELECT count(*) count FROM result_datasets WHERE run_id=40 AND deleted_at_ms=2 AND artifact_id IS NULL AND tombstone_json IS NOT NULL")
        .get_result::<CountRow>(&mut conn).unwrap().count;
    assert_eq!(count, 1);
}

#[test]
fn run_script_message_anchor_is_same_session_and_write_once() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    conn.batch_execute(
        "INSERT INTO script_resources(id,session_id,artifact_id,name,source_kind,content_hash,validation_status,required_aliases_json,outputs_json,diagnostics_json,agent_skill_refs_json,created_at_ms)
         VALUES(30,1,2,'run.rss','manual',printf('%064d',2),'valid','[]','[]','{}','[]',1);
         INSERT INTO simulation_runs(id,session_id,script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,created_at_ms)
         VALUES(40,1,30,2,printf('%064d',2),'run-1','queued','queued','engine-v4','[]',1);
         INSERT INTO messages(id,session_id,run_id,role,kind,content,created_at_ms) VALUES(60,1,40,'assistant','script','code',1),(61,1,40,'assistant','script','code2',1),(62,2,NULL,'assistant','script','foreign',1);
         UPDATE simulation_runs SET script_message_id=60 WHERE id=40;"
    ).unwrap();
    assert!(
        conn.batch_execute("UPDATE simulation_runs SET script_message_id=61 WHERE id=40")
            .is_err()
    );
    conn.batch_execute("INSERT INTO simulation_runs(id,session_id,script_resource_id,script_artifact_id,script_content_hash,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,created_at_ms) VALUES(41,1,30,2,printf('%064d',2),'run-2','queued','queued','engine-v4','[]',1)").unwrap();
    assert!(
        conn.batch_execute("UPDATE simulation_runs SET script_message_id=62 WHERE id=41")
            .is_err()
    );
}

#[test]
fn staging_reservations_cannot_be_negative() {
    let (_root, pool) = database();
    let mut conn = pool.get().unwrap();
    base_rows(&mut conn);
    assert!(
        conn.batch_execute("UPDATE storage_scopes SET reserved_bytes=-1 WHERE id=1")
            .is_err()
    );
    assert!(
        conn.batch_execute("UPDATE storage_scopes SET used_bytes=-1 WHERE id=1")
            .is_err()
    );
}
