//! 数据库层：SQLite 连接池、PRAGMA 与嵌入式迁移。
//!
//! 迁移 SQL 位于 `backend/migrations/`，编译期通过 [`embed_migrations`] 嵌入，
//! 启动时用 [`run_migrations`] 应用。连接池用 r2d2，每个连接在检出时执行
//! `PRAGMA`（WAL / 外键 / busy_timeout / synchronous），见 `docs/db-design.md` §3.2。

pub mod models;
pub mod repositories;
pub mod schema;

use diesel::connection::SimpleConnection;
use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool, PoolError};
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};

/// 编译期嵌入 `backend/migrations/` 下的所有迁移。
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");

/// 连接池类型别名。
pub type DbPool = Pool<ConnectionManager<SqliteConnection>>;

/// 每个池连接在检出时执行的 PRAGMA。
///
/// `diesel::r2d2::Error`（enum）与 `diesel::r2d2::PoolError`（= `r2d2::Error`）是
/// 两个不同类型：customizer 的 `E` 是前者（`ConnectionManager::Error`），`.build()`
/// 与 `pool.get()` 返回的是后者。
#[derive(Debug)]
struct PragmaCustomizer;

impl CustomizeConnection<SqliteConnection, diesel::r2d2::Error> for PragmaCustomizer {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), diesel::r2d2::Error> {
        conn.batch_execute(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;",
        )
        .map_err(diesel::r2d2::Error::QueryError)
    }
}

/// 依据 `DATABASE_URL` 建立连接池。
pub fn establish_pool(database_url: &str) -> Result<DbPool, PoolError> {
    let manager = ConnectionManager::<SqliteConnection>::new(database_url);
    Pool::builder()
        .max_size(8)
        .connection_customizer(Box::new(PragmaCustomizer))
        .build(manager)
}

/// 在给定连接上应用尚未执行的迁移。
pub fn run_migrations(
    conn: &mut SqliteConnection,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    conn.run_pending_migrations(MIGRATIONS)?;
    Ok(())
}

/// 服务进程退出时无法继续执行内存中的任务；启动后把遗留运行态收敛为可解释的失败态。
pub fn recover_interrupted_runs(
    conn: &mut SqliteConnection,
    now_ms: i64,
) -> Result<(), diesel::result::Error> {
    conn.batch_execute(&format!(
        "UPDATE simulation_runs
         SET status='failed', stage='recovery',
             error_json='{{\"code\":\"service_restarted\",\"message\":\"服务重启，运行已中止\"}}',
             finished_at_ms={now_ms}
         WHERE status IN ('queued','running');
         UPDATE model_imports
         SET status='failed',
             error_code='service_restarted',
             error_message='服务重启，模型导入已中止',
             updated_at_ms={now_ms}
         WHERE status='processing';
         UPDATE result_analysis_runs
         SET status='failed', stage='recovery',
             error_json='{{\"code\":\"service_restarted\",\"message\":\"服务重启，运行已中止\"}}',
             finished_at_ms={now_ms}
         WHERE status IN ('pending','running');"
    ))
}
