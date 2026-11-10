//! 已存在 artifact link 的引用计数与用户 storage scope 解析。
//!
//! 调用方决定领域引用集合和事务时机；本模块集中聚合重复 ID，并只在 link 首次创建或最终删除时改变 usage。

use std::collections::BTreeMap;

use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::StorageScopeArtifact;
use crate::db::schema;

use super::{RepoError, RepoResult};

pub(crate) fn storage_scope_id_for_user(
    conn: &mut SqliteConnection,
    user: &UserId,
) -> RepoResult<i32> {
    schema::users::table
        .find(&user.0)
        .select(schema::users::storage_scope_id)
        .first(conn)
        .optional()?
        .ok_or(RepoError::NotFound)
}

pub(crate) fn retain_refs(
    conn: &mut SqliteConnection,
    scope_id: i32,
    artifact_ids: impl IntoIterator<Item = i32>,
) -> RepoResult<()> {
    let counts = reference_counts(artifact_ids)?;
    for (artifact_id, count) in counts {
        retain_ref_count(conn, scope_id, artifact_id, count)?;
    }
    Ok(())
}

pub(crate) fn retain_ref_count(
    conn: &mut SqliteConnection,
    scope_id: i32,
    artifact_id: i32,
    count: i64,
) -> RepoResult<()> {
    if count <= 0 {
        return Err(RepoError::InvalidState(
            "artifact 引用增加次数必须为正数".into(),
        ));
    }
    let link = schema::storage_scope_artifacts::table
        .find((scope_id, artifact_id))
        .first::<StorageScopeArtifact>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    let ref_count = link
        .ref_count
        .checked_add(count)
        .ok_or_else(|| RepoError::InvalidState("artifact 引用计数溢出".into()))?;
    diesel::update(schema::storage_scope_artifacts::table.find((scope_id, artifact_id)))
        .set(schema::storage_scope_artifacts::ref_count.eq(ref_count))
        .execute(conn)?;
    Ok(())
}

pub(crate) fn release_refs(
    conn: &mut SqliteConnection,
    scope_id: i32,
    artifact_ids: impl IntoIterator<Item = i32>,
    now_ms: i64,
) -> RepoResult<()> {
    let counts = reference_counts(artifact_ids)?;
    let mut removed_artifacts = 0_i64;
    let mut removed_bytes = 0_i64;
    for (artifact_id, release_count) in counts {
        let link = schema::storage_scope_artifacts::table
            .find((scope_id, artifact_id))
            .first::<StorageScopeArtifact>(conn)
            .optional()?
            .ok_or(RepoError::NotFound)?;
        if release_count > link.ref_count {
            return Err(RepoError::InvalidState(
                "artifact 引用释放次数超过持有次数".into(),
            ));
        }
        if release_count < link.ref_count {
            diesel::update(schema::storage_scope_artifacts::table.find((scope_id, artifact_id)))
                .set(schema::storage_scope_artifacts::ref_count.eq(link.ref_count - release_count))
                .execute(conn)?;
        } else {
            let size_bytes = schema::artifacts::table
                .find(artifact_id)
                .select(schema::artifacts::size_bytes)
                .first::<i64>(conn)?;
            diesel::delete(schema::storage_scope_artifacts::table.find((scope_id, artifact_id)))
                .execute(conn)?;
            removed_artifacts += 1;
            removed_bytes = removed_bytes
                .checked_add(size_bytes)
                .ok_or_else(|| RepoError::InvalidState("artifact 大小溢出".into()))?;
        }
    }
    if removed_artifacts > 0 {
        diesel::update(schema::storage_scopes::table.find(scope_id))
            .set((
                schema::storage_scopes::used_artifacts
                    .eq(schema::storage_scopes::used_artifacts - removed_artifacts),
                schema::storage_scopes::used_bytes
                    .eq(schema::storage_scopes::used_bytes - removed_bytes),
                schema::storage_scopes::updated_at_ms.eq(now_ms),
            ))
            .execute(conn)?;
    }
    Ok(())
}

fn reference_counts(artifact_ids: impl IntoIterator<Item = i32>) -> RepoResult<BTreeMap<i32, i64>> {
    let mut counts = BTreeMap::<i32, i64>::new();
    for artifact_id in artifact_ids {
        let count = counts.entry(artifact_id).or_default();
        *count = count
            .checked_add(1)
            .ok_or_else(|| RepoError::InvalidState("artifact 引用数量溢出".into()))?;
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;

    use crate::auth::UserId;
    use crate::db::models::{NewArtifact, NewStorageScope, NewUser, StorageScopeArtifact};
    use crate::db::{run_migrations, schema};

    use super::{release_refs, retain_ref_count, retain_refs, storage_scope_id_for_user};
    use crate::db::repositories::RepoError;

    const NOW_MS: i64 = 1_700_000_000_000;
    const ARTIFACT_SIZE: i64 = 17;

    struct Fixture {
        conn: SqliteConnection,
        scope_id: i32,
        artifact_id: i32,
        user: UserId,
    }

    fn fixture() -> Fixture {
        let mut conn = SqliteConnection::establish(":memory:").expect("open in-memory sqlite");
        conn.batch_execute("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");
        run_migrations(&mut conn).expect("run migrations");

        diesel::insert_into(schema::storage_scopes::table)
            .values(NewStorageScope {
                kind: "user".to_string(),
                scope_key: "artifact-ref-tests".to_string(),
                created_at_ms: NOW_MS,
            })
            .execute(&mut conn)
            .expect("insert storage scope");
        let scope_id = schema::storage_scopes::table
            .select(schema::storage_scopes::id)
            .first(&mut conn)
            .expect("read storage scope id");
        let user = UserId("artifact-ref-user".to_string());
        diesel::insert_into(schema::users::table)
            .values(NewUser {
                id: user.0.clone(),
                name: "Artifact Ref User".to_string(),
                password_hash: "test".to_string(),
                storage_scope_id: scope_id,
                created_at_ms: NOW_MS,
            })
            .execute(&mut conn)
            .expect("insert user");
        diesel::insert_into(schema::artifacts::table)
            .values(NewArtifact {
                sha256: "a".repeat(64),
                size_bytes: ARTIFACT_SIZE,
                storage_path: "sha256/aa/artifact".to_string(),
                format: "parquet".to_string(),
                media_type: None,
                created_at_ms: NOW_MS,
            })
            .execute(&mut conn)
            .expect("insert artifact");
        let artifact_id = schema::artifacts::table
            .select(schema::artifacts::id)
            .first(&mut conn)
            .expect("read artifact id");
        diesel::insert_into(schema::storage_scope_artifacts::table)
            .values(StorageScopeArtifact {
                storage_scope_id: scope_id,
                artifact_id,
                ref_count: 1,
                created_at_ms: NOW_MS,
            })
            .execute(&mut conn)
            .expect("link artifact");
        diesel::update(schema::storage_scopes::table.find(scope_id))
            .set((
                schema::storage_scopes::used_artifacts.eq(1_i64),
                schema::storage_scopes::used_bytes.eq(ARTIFACT_SIZE),
            ))
            .execute(&mut conn)
            .expect("account artifact usage");

        Fixture {
            conn,
            scope_id,
            artifact_id,
            user,
        }
    }

    fn ref_count(conn: &mut SqliteConnection, scope_id: i32, artifact_id: i32) -> i64 {
        schema::storage_scope_artifacts::table
            .find((scope_id, artifact_id))
            .select(schema::storage_scope_artifacts::ref_count)
            .first(conn)
            .expect("read ref count")
    }

    fn usage(conn: &mut SqliteConnection, scope_id: i32) -> (i64, i64) {
        schema::storage_scopes::table
            .find(scope_id)
            .select((
                schema::storage_scopes::used_artifacts,
                schema::storage_scopes::used_bytes,
            ))
            .first(conn)
            .expect("read usage")
    }

    #[test]
    fn duplicate_retains_and_releases_preserve_unique_usage() {
        let mut fixture = fixture();

        retain_refs(
            &mut fixture.conn,
            fixture.scope_id,
            [fixture.artifact_id, fixture.artifact_id],
        )
        .expect("retain duplicate references");
        assert_eq!(
            ref_count(&mut fixture.conn, fixture.scope_id, fixture.artifact_id),
            3
        );
        assert_eq!(
            usage(&mut fixture.conn, fixture.scope_id),
            (1, ARTIFACT_SIZE)
        );

        release_refs(
            &mut fixture.conn,
            fixture.scope_id,
            [fixture.artifact_id, fixture.artifact_id],
            NOW_MS + 1,
        )
        .expect("release duplicate references");
        assert_eq!(
            ref_count(&mut fixture.conn, fixture.scope_id, fixture.artifact_id),
            1
        );
        assert_eq!(
            usage(&mut fixture.conn, fixture.scope_id),
            (1, ARTIFACT_SIZE)
        );
    }

    #[test]
    fn retain_batch_rolls_back_when_any_scope_link_is_missing() {
        let mut fixture = fixture();

        let result = fixture
            .conn
            .immediate_transaction::<(), RepoError, _>(|conn| {
                retain_refs(
                    conn,
                    fixture.scope_id,
                    [fixture.artifact_id, fixture.artifact_id + 1],
                )
            });

        assert!(matches!(result, Err(RepoError::NotFound)));
        assert_eq!(
            ref_count(&mut fixture.conn, fixture.scope_id, fixture.artifact_id),
            1
        );
        assert_eq!(
            usage(&mut fixture.conn, fixture.scope_id),
            (1, ARTIFACT_SIZE)
        );
    }

    #[test]
    fn invalid_or_overflowing_reference_counts_fail_closed() {
        let mut fixture = fixture();

        for invalid_count in [0, -1] {
            assert!(matches!(
                retain_ref_count(
                    &mut fixture.conn,
                    fixture.scope_id,
                    fixture.artifact_id,
                    invalid_count,
                ),
                Err(RepoError::InvalidState(_))
            ));
        }
        diesel::update(
            schema::storage_scope_artifacts::table.find((fixture.scope_id, fixture.artifact_id)),
        )
        .set(schema::storage_scope_artifacts::ref_count.eq(i64::MAX))
        .execute(&mut fixture.conn)
        .expect("prepare overflow boundary");
        assert!(matches!(
            retain_ref_count(&mut fixture.conn, fixture.scope_id, fixture.artifact_id, 1,),
            Err(RepoError::InvalidState(_))
        ));
        assert_eq!(
            ref_count(&mut fixture.conn, fixture.scope_id, fixture.artifact_id),
            i64::MAX
        );

        diesel::update(
            schema::storage_scope_artifacts::table.find((fixture.scope_id, fixture.artifact_id)),
        )
        .set(schema::storage_scope_artifacts::ref_count.eq(1_i64))
        .execute(&mut fixture.conn)
        .expect("prepare underflow boundary");
        assert!(matches!(
            release_refs(
                &mut fixture.conn,
                fixture.scope_id,
                [fixture.artifact_id, fixture.artifact_id],
                NOW_MS + 2,
            ),
            Err(RepoError::InvalidState(_))
        ));
        assert_eq!(
            ref_count(&mut fixture.conn, fixture.scope_id, fixture.artifact_id),
            1
        );
        assert_eq!(
            usage(&mut fixture.conn, fixture.scope_id),
            (1, ARTIFACT_SIZE)
        );
    }

    #[test]
    fn user_scope_lookup_hides_missing_users() {
        let mut fixture = fixture();

        assert_eq!(
            storage_scope_id_for_user(&mut fixture.conn, &fixture.user)
                .expect("look up known user"),
            fixture.scope_id
        );
        assert!(matches!(
            storage_scope_id_for_user(
                &mut fixture.conn,
                &UserId("missing-artifact-ref-user".to_string()),
            ),
            Err(RepoError::NotFound)
        ));
    }
}
