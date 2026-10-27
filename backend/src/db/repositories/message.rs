use diesel::prelude::*;
use serde_json::Value;

use crate::auth::UserId;
use crate::db::models::{Message, NewMessage};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct MessageRepo {
    pool: DbPool,
}

impl MessageRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, scope: &UserId, new: NewMessage) -> RepoResult<Message> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Message> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Message, diesel::result::Error, _>(|conn| {
                schema::sessions::table
                    .filter(schema::sessions::id.eq(new.session_id))
                    .filter(schema::sessions::user_id.eq(&scope.0))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)?;
                diesel::insert_into(schema::messages::table)
                    .values(new.clone())
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                if new.role == "user" {
                    let title: String = new.content.chars().take(30).collect();
                    diesel::update(
                        schema::sessions::table
                            .filter(schema::sessions::id.eq(new.session_id))
                            .filter(schema::sessions::title_source.eq("default")),
                    )
                    .set((
                        schema::sessions::title.eq(title),
                        schema::sessions::title_source.eq("auto"),
                        schema::sessions::updated_at_ms.eq(new.created_at_ms),
                    ))
                    .execute(conn)?;
                }
                schema::messages::table.find(id).first(conn)
            })
            .map_err(|error| match error {
                diesel::result::Error::NotFound => RepoError::NotFound,
                other => RepoError::Query(other),
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn upsert_operation(
        &self,
        scope: &UserId,
        operation_id: String,
        new: NewMessage,
    ) -> RepoResult<Message> {
        self.upsert_operation_with_status(scope, operation_id, new)
            .await
            .map(|(message, _)| message)
    }

    pub async fn upsert_operation_with_status(
        &self,
        scope: &UserId,
        operation_id: String,
        new: NewMessage,
    ) -> RepoResult<(Message, bool)> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<(Message, bool)> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<(Message, bool), diesel::result::Error, _>(|conn| {
                schema::sessions::table
                    .filter(schema::sessions::id.eq(new.session_id))
                    .filter(schema::sessions::user_id.eq(&scope.0))
                    .filter(schema::sessions::deletion_status.eq("live"))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)?;
                let existing = schema::messages::table
                    .filter(schema::messages::session_id.eq(new.session_id))
                    .filter(schema::messages::kind.eq(&new.kind))
                    .filter(schema::messages::operation_id.eq(&operation_id))
                    .first::<Message>(conn)
                    .optional()?;
                if let Some(existing) = existing {
                    if existing.run_id == new.run_id
                        && existing.role == new.role
                        && existing.content == new.content
                        && existing.payload_json == new.payload_json
                    {
                        return Ok((existing, false));
                    }
                    if keep_existing_operation(&existing, &new) {
                        return Ok((existing, false));
                    }
                    diesel::update(schema::messages::table.find(existing.id))
                        .set((
                            schema::messages::run_id.eq(new.run_id),
                            schema::messages::role.eq(new.role),
                            schema::messages::content.eq(new.content),
                            schema::messages::payload_json.eq(new.payload_json),
                            schema::messages::revision.eq(existing.revision + 1),
                            schema::messages::created_at_ms.eq(new.created_at_ms),
                        ))
                        .execute(conn)?;
                    return schema::messages::table
                        .find(existing.id)
                        .first(conn)
                        .map(|message| (message, true));
                }
                diesel::insert_into(schema::messages::table)
                    .values((
                        &new,
                        schema::messages::operation_id.eq(Some(operation_id)),
                        schema::messages::revision.eq(1),
                    ))
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                schema::messages::table
                    .find(id)
                    .first(conn)
                    .map(|message| (message, true))
            })
            .map_err(|error| match error {
                diesel::result::Error::NotFound => RepoError::NotFound,
                other => RepoError::Query(other),
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_by_session(
        &self,
        scope: &UserId,
        session_id: i32,
    ) -> RepoResult<Vec<Message>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Message>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let exists = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(scope.0))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .optional()
                .map_err(RepoError::Query)?;
            if exists.is_none() {
                return Err(RepoError::NotFound);
            }
            schema::messages::table
                .filter(schema::messages::session_id.eq(session_id))
                .order(schema::messages::id.asc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_after(
        &self,
        scope: &UserId,
        session_id: i32,
        after_id: i32,
        after_revision: i32,
    ) -> RepoResult<Vec<Message>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Message>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(scope.0))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .map_err(|error| match error {
                    diesel::result::Error::NotFound => RepoError::NotFound,
                    other => RepoError::Query(other),
                })?;

            if after_id > 0 {
                let cursor_exists = schema::messages::table
                    .filter(schema::messages::id.eq(after_id))
                    .filter(schema::messages::session_id.eq(session_id))
                    .filter(schema::messages::revision.ge(after_revision))
                    .select(schema::messages::id)
                    .first::<i32>(&mut conn)
                    .optional()
                    .map_err(RepoError::Query)?
                    .is_some();
                if !cursor_exists {
                    return Err(RepoError::NotFound);
                }
            }

            schema::messages::table
                .filter(schema::messages::session_id.eq(session_id))
                .filter(
                    schema::messages::id
                        .gt(after_id)
                        .or(schema::messages::id
                            .eq(after_id)
                            .and(schema::messages::revision.gt(after_revision)))
                        .or(schema::messages::operation_id.is_not_null().and(
                            schema::messages::id
                                .ne(after_id)
                                .or(schema::messages::revision.gt(after_revision)),
                        )),
                )
                .order(schema::messages::id.asc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn keep_existing_operation(existing: &Message, incoming: &NewMessage) -> bool {
    let Some(existing_payload) = existing
        .payload_json
        .as_deref()
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
    else {
        return false;
    };
    let Some(incoming_payload) = incoming
        .payload_json
        .as_deref()
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
    else {
        return false;
    };

    match incoming.kind.as_str() {
        "study-config" => {
            let existing_revision = existing_payload.get("revision").and_then(Value::as_i64);
            let incoming_revision = incoming_payload.get("revision").and_then(Value::as_i64);
            matches!((existing_revision, incoming_revision), (Some(old), Some(new)) if new <= old)
        }
        "upload-batch" => {
            let existing_revision = existing_payload
                .get("batchRevision")
                .and_then(Value::as_i64);
            let incoming_revision = incoming_payload
                .get("batchRevision")
                .and_then(Value::as_i64);
            if matches!((existing_revision, incoming_revision), (Some(old), Some(new)) if new <= old)
            {
                return true;
            }
            let existing_state = existing_payload.get("state").and_then(Value::as_str);
            let incoming_state = incoming_payload.get("state").and_then(Value::as_str);
            match (existing_state, incoming_state) {
                (Some(old), Some(new)) => {
                    upload_state_terminal(old) && new != old
                        || upload_state_rank(new) < upload_state_rank(old)
                }
                _ => false,
            }
        }
        "workflow-progress" => {
            let existing_position = workflow_position(&existing_payload);
            let incoming_position = workflow_position(&incoming_payload);
            match (existing_position, incoming_position) {
                (Some(old), Some(new)) if new < old => true,
                (Some(old), Some(new)) if new == old => {
                    let existing_state = existing_payload.get("state").and_then(Value::as_str);
                    let incoming_state = incoming_payload.get("state").and_then(Value::as_str);
                    match (existing_state, incoming_state) {
                        (Some(old), Some(new)) => {
                            workflow_state_terminal(old) && new != old
                                || workflow_state_rank(new) < workflow_state_rank(old)
                        }
                        _ => false,
                    }
                }
                _ => false,
            }
        }
        _ => false,
    }
}

fn upload_state_terminal(state: &str) -> bool {
    matches!(state, "confirmed" | "failed" | "discarded" | "expired")
}

fn upload_state_rank(state: &str) -> u8 {
    match state {
        "accepted" | "processing" => 0,
        "awaiting_parse_settings" => 1,
        "awaiting_purpose" => 2,
        "ready" | "ready_to_confirm" => 3,
        "partially_confirmed" => 4,
        "confirmed" | "failed" | "discarded" | "expired" => 5,
        _ => 0,
    }
}

fn workflow_position(payload: &Value) -> Option<(u64, u64)> {
    Some((
        payload.get("executionIndex")?.as_u64()?,
        payload.get("repairIndex")?.as_u64()?,
    ))
}

fn workflow_state_rank(state: &str) -> u8 {
    match state {
        "queued" => 0,
        "generating" => 1,
        "executing" => 2,
        "repairing" => 3,
        "succeeded" | "failed" | "cancelled_by_user" | "service_restarted" => 4,
        _ => 0,
    }
}

fn workflow_state_terminal(state: &str) -> bool {
    matches!(
        state,
        "succeeded" | "failed" | "cancelled_by_user" | "service_restarted"
    )
}
