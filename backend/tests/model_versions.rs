use diesel::connection::SimpleConnection;
use zjudss::application::{default_version_description, normalize_version_description};
use zjudss::auth::UserId;
use zjudss::db::models::DraftRef;
use zjudss::db::repositories::{PublishCommand, VersionRepo};

#[test]
fn model_version_description_defaults_from_canonical_diff() {
    assert_eq!(
        default_version_description(&[], &[("line", "x"), ("bus", "y")]),
        "首次发布：2 张模型表"
    );
    assert_eq!(
        default_version_description(
            &[("bus", "a"), ("line", "a")],
            &[("bus", "b"), ("load", "c")]
        ),
        "新增 1 张表：load；删除 1 张表：line；修改 1 张表：bus"
    );
    assert!(
        normalize_version_description(Some("  调整负荷\r\n并重新计算  "))
            .is_ok_and(|value| value.as_deref() == Some("调整负荷  并重新计算"))
    );
    assert!(normalize_version_description(Some("bad\u{0007}value")).is_err());
    assert!(normalize_version_description(Some(&"界".repeat(201))).is_err());
}

#[tokio::test]
async fn duplicate_model_content_keeps_original_description() {
    let root = tempfile::tempdir().unwrap();
    let pool = zjudss::db::establish_pool(&format!(
        "sqlite://{}",
        root.path().join("versions.db").display()
    ))
    .unwrap();
    zjudss::db::run_migrations(&mut pool.get().unwrap()).unwrap();
    let mut conn = pool.get().unwrap();
    conn.batch_execute(
        r#"
        INSERT INTO storage_scopes(id,kind,scope_key,created_at_ms) VALUES(1,'user','u1',1);
        INSERT INTO users(id,name,password_hash,storage_scope_id,created_at_ms) VALUES('u1','User','hash',1,1);
        INSERT INTO sessions(id,title,created_at_ms,updated_at_ms,user_id,title_source) VALUES(1,'one',1,1,'u1','default');
        INSERT INTO workspaces(id,session_id,created_at_ms,updated_at_ms) VALUES(1,1,1,1);
        INSERT INTO artifacts(id,sha256,size_bytes,storage_path,format,created_at_ms) VALUES(1,printf('%064d',1),10,'a/1','parquet',1);
        INSERT INTO storage_scope_artifacts(storage_scope_id,artifact_id,ref_count,created_at_ms) VALUES(1,1,2,1);
        INSERT INTO workspace_drafts(id,workspace_id,revision,diagnostics_json,created_at_ms,updated_at_ms) VALUES(10,1,1,'{}',1,1);
        INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms)
          VALUES(10,'bus',1,'[]',1,printf('%064d',11),1);
        "#,
    ).unwrap();
    drop(conn);
    let repo = VersionRepo::new(pool.clone());
    let user = UserId("u1".into());
    let first = repo
        .publish(
            &user,
            1,
            PublishCommand {
                expected: DraftRef {
                    id: 10,
                    revision: 1,
                },
                client_publish_id: "p1".into(),
                description: "原描述".into(),
                engine_contract: "engine-v4".into(),
                now_ms: 2,
            },
        )
        .await
        .unwrap();
    assert!(!first.deduplicated && !first.replayed);
    let replay = repo
        .publish(
            &user,
            1,
            PublishCommand {
                expected: DraftRef {
                    id: 10,
                    revision: 1,
                },
                client_publish_id: "p1".into(),
                description: "被忽略".into(),
                engine_contract: "engine-v4".into(),
                now_ms: 3,
            },
        )
        .await
        .unwrap();
    assert!(replay.replayed);

    let mut conn = pool.get().unwrap();
    conn.batch_execute(
        "INSERT INTO workspace_drafts(id,workspace_id,base_version_id,revision,diagnostics_json,created_at_ms,updated_at_ms) VALUES(11,1,1,1,'{}',3,3);
         INSERT INTO draft_dataframes(draft_id,table_name,artifact_id,schema_json,row_count,content_hash,created_at_ms) VALUES(11,'bus',1,'[]',1,printf('%064d',11),3);"
    ).unwrap();
    drop(conn);
    let duplicate = repo
        .publish(
            &user,
            1,
            PublishCommand {
                expected: DraftRef {
                    id: 11,
                    revision: 1,
                },
                client_publish_id: "p2".into(),
                description: "新描述".into(),
                engine_contract: "engine-v4".into(),
                now_ms: 4,
            },
        )
        .await
        .unwrap();
    assert!(duplicate.deduplicated && !duplicate.replayed);
    assert_eq!(duplicate.version_id, first.version_id);
    let version = repo.find_version(&user, first.version_id).await.unwrap();
    assert_eq!(version.version.description, "原描述");
}
