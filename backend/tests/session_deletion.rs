use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use zjudss::auth::UserId;
use zjudss::db::repositories::SessionRepo;

#[tokio::test]
async fn deletion_impact_and_cleanup_include_each_chart_reference() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("session.db").display());
    let pool = zjudss::db::establish_pool(&url).unwrap();
    zjudss::db::run_migrations(&mut pool.get().unwrap()).unwrap();
    pool.get()
        .unwrap()
        .batch_execute(
            r#"
            INSERT INTO storage_scopes(id,kind,scope_key,used_artifacts,used_bytes,created_at_ms,updated_at_ms)
              VALUES(1,'user','u1',2,30,1,1);
            INSERT INTO users(id,name,password_hash,storage_scope_id,created_at_ms)
              VALUES('u1','One','x',1,1);
            INSERT INTO sessions(id,title,created_at_ms,updated_at_ms,user_id,title_source)
              VALUES(1,'one',1,1,'u1','default');
            INSERT INTO artifacts(id,sha256,size_bytes,storage_path,format,media_type,created_at_ms)
              VALUES(1,printf('%064d',1),10,'objects/source','parquet',NULL,1),
                    (2,printf('%064d',2),20,'objects/chart','parquet',NULL,1);
            INSERT INTO storage_scope_artifacts(storage_scope_id,artifact_id,ref_count,created_at_ms)
              VALUES(1,1,1,1),(1,2,1,1);
            INSERT INTO simulation_runs(id,session_id,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,run_kind,created_at_ms)
              VALUES(1,1,'run-1','succeeded','done','v4','[]','managed',1);
            INSERT INTO result_datasets(id,run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms)
              VALUES(1,1,1,'source','[]',printf('%064d',3),2,10,1);
            INSERT INTO dataset_views(
              id,session_id,source_dataset_id,artifact_id,client_operation_id,request_hash,name,kind,
              config_json,config_hash,chart_contract_version,diagnostics_json,plot_schema_json,
              plot_schema_hash,plot_content_hash,row_count,point_count,size_bytes,source_name,
              source_schema_json,source_schema_hash,source_content_hash,created_at_ms)
            VALUES(1,1,1,2,'op','request','chart','line','{}',printf('%064d',4),'chart-model-v1',
              '{}','[]',printf('%064d',5),printf('%064d',2),2,2,20,'source','[]',printf('%064d',3),printf('%064d',1),2);
            "#,
        )
        .unwrap();

    let repo = SessionRepo::new(pool.clone());
    let user = UserId("u1".into());
    let token = "a".repeat(64);
    let impact = repo
        .preview_deletion(&user, 1, token.clone(), 100, 2)
        .await
        .unwrap();
    assert_eq!(impact.chart_count, 1);
    assert_eq!(impact.chart_bytes, 20);
    assert_eq!(impact.logical_bytes, 30);
    assert!(repo.begin_delete(&user, 1, token, 3).await.unwrap());
    repo.delete(&user, 1).await.unwrap();

    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut conn = pool.get().unwrap();
    assert_eq!(
        diesel::sql_query("SELECT count(*) count FROM dataset_views")
            .get_result::<Count>(&mut conn)
            .unwrap()
            .count,
        0
    );
    assert_eq!(
        diesel::sql_query("SELECT count(*) count FROM storage_scope_artifacts")
            .get_result::<Count>(&mut conn)
            .unwrap()
            .count,
        0
    );
}
