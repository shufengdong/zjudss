use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use tempfile::TempDir;
use zjudss::auth::UserId;
use zjudss::db::repositories::{ArtifactDraft, ChartViewDraft, ChartViewRepo, RepoError};

fn database() -> (TempDir, zjudss::db::DbPool) {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("chart.db").display());
    let pool = zjudss::db::establish_pool(&url).unwrap();
    zjudss::db::run_migrations(&mut pool.get().unwrap()).unwrap();
    (root, pool)
}

fn seed(pool: &zjudss::db::DbPool) {
    pool.get()
        .unwrap()
        .batch_execute(
            r#"
            INSERT INTO storage_scopes(id,kind,scope_key,used_artifacts,used_bytes,created_at_ms,updated_at_ms)
              VALUES(1,'user','u1',1,10,1,1),(2,'user','u2',0,0,1,1);
            INSERT INTO users(id,name,password_hash,storage_scope_id,created_at_ms)
              VALUES('u1','One','x',1,1),('u2','Two','x',2,1);
            INSERT INTO sessions(id,title,created_at_ms,updated_at_ms,user_id,title_source)
              VALUES(1,'one',1,1,'u1','default'),(2,'two',1,1,'u2','default');
            INSERT INTO artifacts(id,sha256,size_bytes,storage_path,format,media_type,created_at_ms)
              VALUES(1,printf('%064d',1),10,'objects/00/source','parquet','application/vnd.apache.parquet',1);
            INSERT INTO storage_scope_artifacts(storage_scope_id,artifact_id,ref_count,created_at_ms)
              VALUES(1,1,1,1);
            INSERT INTO simulation_runs(id,session_id,client_run_id,status,stage,engine_contract_version,agent_skill_refs_json,run_kind,created_at_ms)
              VALUES(1,1,'run-1','succeeded','done','v4','[]','managed',1);
            INSERT INTO result_datasets(id,run_id,artifact_id,result_name,schema_json,schema_hash,row_count,size_bytes,created_at_ms)
              VALUES(1,1,1,'voltages','[]',printf('%064d',3),2,10,1);
            "#,
        )
        .unwrap();
}

fn draft(operation: &str, request_hash: &str, name: &str, sha: &str) -> ChartViewDraft {
    ChartViewDraft {
        source_dataset_id: 1,
        client_operation_id: operation.into(),
        request_hash: request_hash.into(),
        name: name.into(),
        kind: "line".into(),
        config_json: r#"{"kind":"line"}"#.into(),
        config_hash: "c".repeat(64),
        chart_contract_version: "chart-model-v1".into(),
        diagnostics_json: "{}".into(),
        plot_schema_json: "[]".into(),
        plot_schema_hash: "s".repeat(64),
        plot_content_hash: sha.into(),
        row_count: 2,
        point_count: 2,
        size_bytes: 20,
        created_at_ms: 10,
        artifact: ArtifactDraft {
            sha256: sha.into(),
            size_bytes: 20,
            storage_path: format!("objects/{sha}"),
            format: "parquet".into(),
            media_type: Some("application/vnd.apache.parquet".into()),
            created_at_ms: 10,
        },
    }
}

#[derive(QueryableByName)]
struct NameRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
}

#[test]
fn initial_schema_has_only_the_immutable_chart_columns() {
    let (_root, pool) = database();
    let columns = diesel::sql_query("PRAGMA table_info(dataset_views)")
        .load::<NameRow>(&mut pool.get().unwrap())
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect::<Vec<_>>();
    assert!(columns.contains(&"session_id".into()));
    assert!(columns.contains(&"plot_content_hash".into()));
    assert!(columns.contains(&"tombstone_json".into()));
    assert!(!columns.contains(&"view_dataset_id".into()));
    assert!(!columns.contains(&"spec_json".into()));
    assert!(!columns.contains(&"is_default".into()));
}

#[tokio::test]
async fn publish_replay_delete_and_source_tombstone_are_user_scoped_and_atomic() {
    let (_root, pool) = database();
    seed(&pool);
    let repo = ChartViewRepo::new(pool.clone());
    let u1 = UserId("u1".into());
    let u2 = UserId("u2".into());
    let sha = "2".repeat(64);

    let first = repo
        .publish(&u1, draft("op-1", "r1", "A", &sha), |_| Ok(()))
        .await
        .unwrap();
    assert!(!first.replayed);
    let replay = repo
        .publish(&u1, draft("op-1", "r1", "ignored", &sha), |_| {
            panic!("replay must not publish")
        })
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.view.id, first.view.id);
    assert!(matches!(
        repo.publish(&u1, draft("op-1", "different", "A", &sha), |_| Ok(()))
            .await,
        Err(RepoError::IdempotencyConflict)
    ));
    assert!(matches!(
        repo.find(&u2, first.view.id).await,
        Err(RepoError::NotFound)
    ));
    assert!(matches!(
        repo.publish(&u1, draft("op-name", "rn", "A", &sha), |_| Ok(()))
            .await,
        Err(RepoError::InvalidState(code)) if code == "chart_name_conflict"
    ));
    let lowercase = repo
        .publish(&u1, draft("op-lower", "rl", "a", &sha), |_| Ok(()))
        .await
        .unwrap();
    let listed = repo.list(&u1, 1, None, 100).await.unwrap();
    assert_eq!(
        listed.iter().map(|view| view.id).collect::<Vec<_>>(),
        [lowercase.view.id, first.view.id]
    );

    let deleted = repo.delete(&u1, first.view.id, 30).await.unwrap();
    assert_eq!(deleted.artifact_id, first.view.artifact_id.unwrap());
    assert!(matches!(
        repo.find(&u1, first.view.id).await,
        Err(RepoError::NotFound)
    ));
    assert!(matches!(
        repo.publish(&u1, draft("op-1", "changed", "A", &sha), |_| Ok(()))
            .await,
        Err(RepoError::InvalidState(code)) if code == "chart_operation_gone"
    ));
    let second = repo
        .publish(&u1, draft("op-2", "r2", "A", &sha), |_| Ok(()))
        .await
        .unwrap();
    pool.get()
        .unwrap()
        .batch_execute(
            "UPDATE result_datasets SET artifact_id=NULL,deleted_at_ms=40,tombstone_json='{}' WHERE id=1",
        )
        .unwrap();
    assert!(repo.find(&u1, second.view.id).await.unwrap().source_deleted);
}

#[tokio::test]
async fn failed_artifact_finalize_rolls_back_view_and_logical_reference() {
    let (_root, pool) = database();
    seed(&pool);
    let repo = ChartViewRepo::new(pool.clone());
    let sha = "4".repeat(64);
    let result = repo
        .publish(
            &UserId("u1".into()),
            draft("op-fail", "rf", "failure", &sha),
            |_| Err(RepoError::Storage("injected".into())),
        )
        .await;
    assert!(matches!(result, Err(RepoError::Storage(_))));
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }
    let mut conn = pool.get().unwrap();
    let views = diesel::sql_query("SELECT count(*) count FROM dataset_views")
        .get_result::<Count>(&mut conn)
        .unwrap()
        .count;
    let links = diesel::sql_query("SELECT count(*) count FROM storage_scope_artifacts")
        .get_result::<Count>(&mut conn)
        .unwrap()
        .count;
    assert_eq!(views, 0);
    assert_eq!(links, 1);
}
