//! 数据访问层：把 Diesel 的同步查询包装成 async，并对上层暴露领域语义。
//!
//! 每个 repository 持有连接池克隆，方法内部用 `spawn_blocking` 跑同步 Diesel，
//! 不阻塞 async worker（见 `docs/db-design.md` §3.3）。

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::{Integer, Text};

use super::models::{Message, NewMessage, NewSession, NewSimulationTask, Session, SimulationTask};
use super::schema;
use super::DbPool;

/// 仓储层统一错误：连接池 / 查询 / 阻塞任务 / 资源不存在四类来源。
#[derive(Debug)]
pub enum RepoError {
    Pool(diesel::r2d2::PoolError),
    Query(diesel::result::Error),
    Join(tokio::task::JoinError),
    /// 目标行不存在（供上层映射为 404）。
    NotFound,
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Pool(e) => write!(f, "连接池错误: {e}"),
            RepoError::Query(e) => write!(f, "查询错误: {e}"),
            RepoError::Join(e) => write!(f, "阻塞任务错误: {e}"),
            RepoError::NotFound => write!(f, "资源不存在"),
        }
    }
}

impl std::error::Error for RepoError {}

pub type RepoResult<T> = Result<T, RepoError>;

#[derive(Clone)]
pub struct SessionRepo {
    pool: DbPool,
}

impl SessionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, title: &str, now_ms: i64) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::insert_into(schema::sessions::table)
                .values(NewSession {
                    title,
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                })
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            let id: i32 = diesel::dsl::sql::<diesel::sql_types::Integer>(
                "SELECT last_insert_rowid()",
            )
            .get_result(&mut conn)
            .map_err(RepoError::Query)?;
            schema::sessions::table
                .find(id)
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(&self) -> RepoResult<Vec<Session>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .order(schema::sessions::updated_at_ms.desc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn touch(&self, session_id: i32, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::update(schema::sessions::table.find(session_id))
                .set(schema::sessions::updated_at_ms.eq(now_ms))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, session_id: i32) -> RepoResult<Option<Session>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .find(session_id)
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(&self, session_id: i32, title: &str, now_ms: i64) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let updated = diesel::update(schema::sessions::table.find(session_id))
                .set((
                    schema::sessions::title.eq(title),
                    schema::sessions::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            schema::sessions::table
                .find(session_id)
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    /// 删除会话；子表（messages/simulation_tasks/task_files/...）靠 FK `ON DELETE CASCADE`
    /// 级联清理（依赖 PRAGMA `foreign_keys=ON`）。
    pub async fn delete(&self, session_id: i32) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let deleted = diesel::delete(schema::sessions::table.find(session_id))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if deleted == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }
}

#[derive(Clone)]
pub struct MessageRepo {
    pool: DbPool,
}

impl MessageRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewMessage) -> RepoResult<Message> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Message> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::insert_into(schema::messages::table)
                .values(new)
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            let id: i32 = diesel::dsl::sql::<diesel::sql_types::Integer>(
                "SELECT last_insert_rowid()",
            )
            .get_result(&mut conn)
            .map_err(RepoError::Query)?;
            schema::messages::table
                .find(id)
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_by_session(&self, session_id: i32) -> RepoResult<Vec<Message>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Message>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::messages::table
                .filter(schema::messages::session_id.eq(session_id))
                .order(schema::messages::created_at_ms.asc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }
}

#[derive(Clone)]
pub struct TaskRepo {
    pool: DbPool,
}

/// 批量「每会话最新任务状态」查询的行形状（`diesel::sql_query` 用）。
#[derive(diesel::QueryableByName)]
struct LatestTaskRow {
    #[diesel(sql_type = Integer)]
    session_id: i32,
    #[diesel(sql_type = Text)]
    status: String,
}

impl TaskRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewSimulationTask) -> RepoResult<SimulationTask> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<SimulationTask> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::insert_into(schema::simulation_tasks::table)
                .values(new)
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            let id: i32 = diesel::dsl::sql::<diesel::sql_types::Integer>(
                "SELECT last_insert_rowid()",
            )
            .get_result(&mut conn)
            .map_err(RepoError::Query)?;
            schema::simulation_tasks::table
                .find(id)
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    /// 状态机 → running：记录开始时间与阶段。
    pub async fn mark_running(&self, task_id: i32, stage: &str, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::update(schema::simulation_tasks::table.find(task_id))
                .set((
                    schema::simulation_tasks::status.eq("running"),
                    schema::simulation_tasks::stage.eq(stage),
                    schema::simulation_tasks::started_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    /// 状态机 → 终态（succeeded/failed）：记录结束时间、退出码与结构化错误。
    pub async fn mark_done(
        &self,
        task_id: i32,
        status: &str,
        stage: &str,
        exit_code: Option<i32>,
        error_json: Option<String>,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::update(schema::simulation_tasks::table.find(task_id))
                .set((
                    schema::simulation_tasks::status.eq(status),
                    schema::simulation_tasks::stage.eq(stage),
                    schema::simulation_tasks::exit_code.eq(exit_code),
                    schema::simulation_tasks::error_json.eq(error_json),
                    schema::simulation_tasks::finished_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, task_id: i32) -> RepoResult<Option<SimulationTask>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<SimulationTask>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::simulation_tasks::table
                .find(task_id)
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    /// 某会话最新任务的状态（无任务时 `None`）。
    pub async fn latest_status_for_session(&self, session_id: i32) -> RepoResult<Option<String>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<String>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::simulation_tasks::table
                .filter(schema::simulation_tasks::session_id.eq(session_id))
                .order(schema::simulation_tasks::id.desc())
                .select(schema::simulation_tasks::status)
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    /// 全量「每会话最新任务状态」一次查询（供 `GET /sessions` 列表推导状态）。
    pub async fn latest_statuses(&self) -> RepoResult<HashMap<i32, String>> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<HashMap<i32, String>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let rows: Vec<LatestTaskRow> = diesel::sql_query(
                "SELECT t.session_id, t.status FROM simulation_tasks t \
                 JOIN (SELECT session_id, MAX(id) AS max_id \
                       FROM simulation_tasks GROUP BY session_id) m \
                 ON t.id = m.max_id",
            )
            .load(&mut conn)
            .map_err(RepoError::Query)?;
            Ok(rows.into_iter().map(|r| (r.session_id, r.status)).collect())
        })
        .await
        .map_err(RepoError::Join)?
    }
}
