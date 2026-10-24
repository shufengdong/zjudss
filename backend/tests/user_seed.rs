use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use tempfile::TempDir;
use zjudss::application::StorageLimits;
use zjudss::auth::{INIT_USERS_CSV, seed_users};
use zjudss::db;

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct UserRow {
    #[diesel(sql_type = Text)]
    id: String,
    #[diesel(sql_type = Text)]
    password_hash: String,
}

#[derive(QueryableByName)]
struct ColumnRow {
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = BigInt)]
    is_not_null: i64,
}

fn database() -> (TempDir, db::DbPool) {
    let root = tempfile::tempdir().expect("create database root");
    let pool = db::establish_pool(&root.path().join("seed.db").display().to_string())
        .expect("create database pool");
    db::run_migrations(&mut pool.get().expect("checkout database")).expect("run migrations");
    (root, pool)
}

fn count(conn: &mut SqliteConnection, table: &str) -> i64 {
    diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
        .get_result::<CountRow>(conn)
        .expect("count rows")
        .count
}

#[test]
fn embedded_csv_seeds_all_users_with_argon2id_hashes_and_scopes() {
    let (_root, pool) = database();
    let expected = std::str::from_utf8(INIT_USERS_CSV)
        .expect("embedded users are UTF-8")
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .count();

    let summary = common::seed_test_users(&pool, INIT_USERS_CSV);
    let mut conn = pool.get().expect("checkout database");

    assert_eq!(summary.user_count, expected);
    assert!(summary.initialized);
    assert_eq!(count(&mut conn, "users"), expected as i64);
    assert_eq!(count(&mut conn, "storage_scopes"), expected as i64);
    let users = diesel::sql_query("SELECT id,password_hash FROM users ORDER BY id")
        .load::<UserRow>(&mut conn)
        .expect("read users");
    assert!(users.iter().all(|user| {
        user.password_hash.starts_with("$argon2id$") && user.password_hash != user.id
    }));
}

#[test]
fn seed_validation_and_insert_failures_leave_no_partial_rows() {
    let invalid: &[&[u8]] = &[
        b"ID,NAME\n",
        b"ID,NAME\n1001,Alice\n1001,Bob\n",
        b"id,NAME\n1001,Alice\n",
        b"ID,NAME,ROLE\n1001,Alice,admin\n",
        b"ID,NAME\n1001,Ali\x01ce\n",
        b"ID,NAME\n,Missing id\n",
    ];

    for csv in invalid {
        let (_root, pool) = database();
        let mut conn = pool.get().expect("checkout database");
        let error = seed_users(&mut conn, csv, 1_700_000_000_000, StorageLimits::default())
            .expect_err("invalid seed must fail");
        assert_eq!(error.code(), "invalid_user_seed");
        assert_eq!(count(&mut conn, "users"), 0);
        assert_eq!(count(&mut conn, "storage_scopes"), 0);
        assert_eq!(count(&mut conn, "system_metadata"), 0);
    }

    let (_root, pool) = database();
    let mut conn = pool.get().expect("checkout database");
    let invalid_limits = StorageLimits {
        artifact_limit: 0,
        ..StorageLimits::default()
    };
    assert!(seed_users(&mut conn, b"ID,NAME\n1001,Alice\n", 1, invalid_limits).is_err());
    assert_eq!(count(&mut conn, "users"), 0);
    assert_eq!(count(&mut conn, "storage_scopes"), 0);

    let (_root, pool) = database();
    let mut conn = pool.get().expect("checkout database");
    diesel::sql_query(
        "INSERT INTO storage_scopes(kind,scope_key,created_at_ms) VALUES('user','1002',1)",
    )
    .execute(&mut conn)
    .expect("prepare conflicting second scope");
    assert!(
        seed_users(
            &mut conn,
            b"ID,NAME\n1001,Alice\n1002,Bob\n",
            2,
            StorageLimits::default(),
        )
        .is_err()
    );
    assert_eq!(count(&mut conn, "users"), 0);
    assert_eq!(count(&mut conn, "storage_scopes"), 1);
    assert_eq!(count(&mut conn, "system_metadata"), 0);
}

#[test]
fn identical_seed_is_idempotent_and_changed_seed_requires_reset() {
    let (_root, pool) = database();
    let mut conn = pool.get().expect("checkout database");
    let csv = b"ID,NAME\n1001,Alice\n1002,Bob\n";

    let first = seed_users(&mut conn, csv, 100, StorageLimits::default()).expect("first seed");
    let second = seed_users(&mut conn, csv, 200, StorageLimits::default()).expect("repeat seed");
    assert!(first.initialized);
    assert!(!second.initialized);
    assert_eq!(first.seed_sha256, second.seed_sha256);
    assert_eq!(count(&mut conn, "users"), 2);
    assert_eq!(count(&mut conn, "storage_scopes"), 2);

    let error = seed_users(
        &mut conn,
        b"ID,NAME\n1001,Alice changed\n1002,Bob\n",
        300,
        StorageLimits::default(),
    )
    .expect_err("changed seed must require reset");
    assert_eq!(error.code(), "user_seed_mismatch_reset_required");
    assert_eq!(count(&mut conn, "users"), 2);
}

#[test]
fn sessions_require_user_id_and_drop_generic_scope_columns() {
    let (_root, pool) = database();
    let mut conn = pool.get().expect("checkout database");
    let columns = diesel::sql_query(
        "SELECT name, \"notnull\" AS is_not_null FROM pragma_table_info('sessions') ORDER BY cid",
    )
    .load::<ColumnRow>(&mut conn)
    .expect("read session columns");

    let user_id = columns.iter().find(|column| column.name == "user_id");
    assert_eq!(user_id.map(|column| column.is_not_null), Some(1));
    assert!(
        !columns
            .iter()
            .any(|column| column.name == "access_scope_kind")
    );
    assert!(
        !columns
            .iter()
            .any(|column| column.name == "access_scope_key")
    );
    assert!(
        !columns
            .iter()
            .any(|column| column.name == "storage_scope_id")
    );
}
mod common;
