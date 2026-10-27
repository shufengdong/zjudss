use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{
    StudyConfigurationRecord, StudyInputBindingRecord, StudyResultSelectionRecord,
    StudyStructValueRecord,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone)]
pub struct StudyConfigurationAggregate {
    pub configuration: StudyConfigurationRecord,
    pub bindings: Vec<StudyInputBindingRecord>,
    pub structs: Vec<StudyStructValueRecord>,
    pub results: Vec<StudyResultSelectionRecord>,
}

#[derive(Debug, Clone)]
pub struct NewStudyConfiguration {
    pub session_id: i32,
    pub facade_id: String,
    pub facade_hash: String,
    pub engine_snapshot_id: String,
    pub status: String,
    pub intent_json: String,
    pub bindings: Vec<StudyInputBindingRecord>,
    pub structs: Vec<StudyStructValueRecord>,
    pub results: Vec<StudyResultSelectionRecord>,
    pub now_ms: i64,
}

#[derive(Clone)]
pub struct StudyConfigurationRepo {
    pool: DbPool,
}

impl StudyConfigurationRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        user: &UserId,
        input: NewStudyConfiguration,
    ) -> RepoResult<StudyConfigurationAggregate> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                ensure_session(conn, &user, input.session_id)?;
                diesel::insert_into(schema::study_configurations::table)
                    .values((
                        schema::study_configurations::session_id.eq(input.session_id),
                        schema::study_configurations::facade_id.eq(input.facade_id),
                        schema::study_configurations::facade_hash.eq(input.facade_hash),
                        schema::study_configurations::engine_snapshot_id
                            .eq(input.engine_snapshot_id),
                        schema::study_configurations::status.eq(input.status),
                        schema::study_configurations::revision.eq(1_i64),
                        schema::study_configurations::intent_json.eq(input.intent_json),
                        schema::study_configurations::confirmed_at_ms.eq(Option::<i64>::None),
                        schema::study_configurations::created_at_ms.eq(input.now_ms),
                        schema::study_configurations::updated_at_ms.eq(input.now_ms),
                    ))
                    .execute(conn)?;
                let id = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
                    "last_insert_rowid()",
                ))
                .get_result::<i32>(conn)?;
                insert_children(conn, id, input.bindings, input.structs, input.results)?;
                load_owned(conn, &user, id)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user: &UserId, id: i32) -> RepoResult<StudyConfigurationAggregate> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            load_owned(&mut conn, &user, id)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn latest_confirmed_resource(
        &self,
        user: &UserId,
        session_id: i32,
        facade_id: &str,
        input_name: &str,
    ) -> RepoResult<Option<String>> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        let facade_id = facade_id.to_string();
        let input_name = input_name.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_session(&mut conn, &user, session_id)?;
            schema::study_input_bindings::table
                .inner_join(
                    schema::study_configurations::table.on(schema::study_configurations::id
                        .eq(schema::study_input_bindings::configuration_id)),
                )
                .filter(schema::study_configurations::session_id.eq(session_id))
                .filter(schema::study_configurations::facade_id.eq(facade_id))
                .filter(schema::study_configurations::status.eq("confirmed"))
                .filter(schema::study_input_bindings::input_name.eq(input_name))
                .filter(schema::study_input_bindings::resource_id.is_not_null())
                .order(schema::study_configurations::confirmed_at_ms.desc())
                .select(schema::study_input_bindings::resource_id)
                .first::<Option<String>>(&mut conn)
                .optional()
                .map(Option::flatten)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replace(
        &self,
        user: &UserId,
        id: i32,
        expected_revision: i64,
        facade_id: String,
        facade_hash: String,
        engine_snapshot_id: String,
        status: String,
        bindings: Vec<StudyInputBindingRecord>,
        structs: Vec<StudyStructValueRecord>,
        results: Vec<StudyResultSelectionRecord>,
        now_ms: i64,
    ) -> RepoResult<StudyConfigurationAggregate> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let current = load_owned(conn, &user, id)?;
                if current.configuration.revision != expected_revision
                    || matches!(
                        current.configuration.status.as_str(),
                        "confirmed" | "invalidated" | "cancelled"
                    )
                {
                    return Err(RepoError::InvalidState("configuration_stale".into()));
                }
                diesel::delete(
                    schema::study_input_bindings::table
                        .filter(schema::study_input_bindings::configuration_id.eq(id)),
                )
                .execute(conn)?;
                diesel::delete(
                    schema::study_struct_values::table
                        .filter(schema::study_struct_values::configuration_id.eq(id)),
                )
                .execute(conn)?;
                diesel::delete(
                    schema::study_result_selections::table
                        .filter(schema::study_result_selections::configuration_id.eq(id)),
                )
                .execute(conn)?;
                insert_children(conn, id, bindings, structs, results)?;
                diesel::update(schema::study_configurations::table.find(id))
                    .set((
                        schema::study_configurations::facade_id.eq(facade_id),
                        schema::study_configurations::facade_hash.eq(facade_hash),
                        schema::study_configurations::engine_snapshot_id.eq(engine_snapshot_id),
                        schema::study_configurations::status.eq(status),
                        schema::study_configurations::revision.eq(expected_revision + 1),
                        schema::study_configurations::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                load_owned(conn, &user, id)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn cancel(
        &self,
        user: &UserId,
        id: i32,
        expected_revision: i64,
        now_ms: i64,
    ) -> RepoResult<StudyConfigurationAggregate> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let current = load_owned(conn, &user, id)?;
                if current.configuration.revision != expected_revision
                    || matches!(
                        current.configuration.status.as_str(),
                        "confirmed" | "invalidated" | "cancelled"
                    )
                {
                    return Err(RepoError::InvalidState("configuration_stale".into()));
                }
                diesel::update(schema::study_configurations::table.find(id))
                    .set((
                        schema::study_configurations::status.eq("cancelled"),
                        schema::study_configurations::revision.eq(expected_revision + 1),
                        schema::study_configurations::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                load_owned(conn, &user, id)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn ensure_session(conn: &mut SqliteConnection, user: &str, session_id: i32) -> RepoResult<()> {
    schema::sessions::table
        .filter(schema::sessions::id.eq(session_id))
        .filter(schema::sessions::user_id.eq(user))
        .select(schema::sessions::id)
        .first::<i32>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    Ok(())
}

fn load_owned(
    conn: &mut SqliteConnection,
    user: &str,
    id: i32,
) -> RepoResult<StudyConfigurationAggregate> {
    let configuration = schema::study_configurations::table
        .inner_join(
            schema::sessions::table
                .on(schema::sessions::id.eq(schema::study_configurations::session_id)),
        )
        .filter(schema::study_configurations::id.eq(id))
        .filter(schema::sessions::user_id.eq(user))
        .select(schema::study_configurations::all_columns)
        .first::<StudyConfigurationRecord>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    let bindings = schema::study_input_bindings::table
        .filter(schema::study_input_bindings::configuration_id.eq(id))
        .order(schema::study_input_bindings::ordinal.asc())
        .load(conn)?;
    let structs = schema::study_struct_values::table
        .filter(schema::study_struct_values::configuration_id.eq(id))
        .order(schema::study_struct_values::input_name.asc())
        .load(conn)?;
    let results = schema::study_result_selections::table
        .filter(schema::study_result_selections::configuration_id.eq(id))
        .order(schema::study_result_selections::ordinal.asc())
        .load(conn)?;
    Ok(StudyConfigurationAggregate {
        configuration,
        bindings,
        structs,
        results,
    })
}

fn insert_children(
    conn: &mut SqliteConnection,
    configuration_id: i32,
    mut bindings: Vec<StudyInputBindingRecord>,
    mut structs: Vec<StudyStructValueRecord>,
    mut results: Vec<StudyResultSelectionRecord>,
) -> RepoResult<()> {
    for row in &mut bindings {
        row.configuration_id = configuration_id;
    }
    for row in &mut structs {
        row.configuration_id = configuration_id;
    }
    for row in &mut results {
        row.configuration_id = configuration_id;
    }
    if !bindings.is_empty() {
        diesel::insert_into(schema::study_input_bindings::table)
            .values(bindings)
            .execute(conn)?;
    }
    if !structs.is_empty() {
        diesel::insert_into(schema::study_struct_values::table)
            .values(structs)
            .execute(conn)?;
    }
    if !results.is_empty() {
        diesel::insert_into(schema::study_result_selections::table)
            .values(results)
            .execute(conn)?;
    }
    Ok(())
}
