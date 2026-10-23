use super::{RepoError, RepoResult};
use crate::{
    auth::UserId,
    db::{DbPool, models::Message, schema},
};
use diesel::{
    prelude::*,
    sql_query,
    sql_types::{BigInt, Double, Integer, Nullable, Text},
};
#[derive(Debug, Clone)]
pub struct SearchFilters {
    pub script_name: Option<String>,
    pub status: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryHit {
    pub session_id: String,
    pub title: String,
    pub updated_at_ms: i64,
    pub score: f64,
    pub snippet: String,
}
#[derive(QueryableByName)]
struct HitRow {
    #[diesel(sql_type=Integer)]
    session_id: i32,
    #[diesel(sql_type=Text)]
    title: String,
    #[diesel(sql_type=BigInt)]
    updated_at_ms: i64,
    #[diesel(sql_type=Double)]
    score: f64,
    #[diesel(sql_type=Text)]
    snippet: String,
}
#[derive(Clone)]
pub struct HistoryRepo {
    pool: DbPool,
}
impl HistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
    pub async fn fts_available(&self) -> bool {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move||pool.get().ok().and_then(|mut c|sql_query("SELECT count(*) AS value FROM chat_search_fts WHERE chat_search_fts MATCH 'test'").load::<CountRow>(&mut c).ok()).is_some()).await.unwrap_or(false)
    }
    pub async fn search(
        &self,
        scope: &UserId,
        query: String,
        filters: SearchFilters,
        limit: i64,
    ) -> RepoResult<Vec<HistoryHit>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn=pool.get().map_err(RepoError::Pool)?;
            let limit=limit.clamp(1,20);
            let rows=if query.chars().count()<=2 {
                let pattern=format!("%{}%",escape_like(&query));
                sql_query("SELECT s.id AS session_id,s.title,s.updated_at_ms,0.0 AS score,d.search_text AS snippet FROM chat_search_documents d JOIN sessions s ON s.id=d.session_id WHERE s.user_id=? AND d.search_text LIKE ? ESCAPE '\\' AND (? IS NULL OR s.updated_at_ms>=?) AND (? IS NULL OR s.updated_at_ms<=?) AND (? IS NULL OR EXISTS(SELECT 1 FROM simulation_runs r JOIN script_resources sr ON sr.id=r.script_resource_id WHERE r.session_id=s.id AND sr.name=? AND r.deleted_at_ms IS NULL)) AND (? IS NULL OR EXISTS(SELECT 1 FROM simulation_runs r WHERE r.session_id=s.id AND r.status=? AND r.deleted_at_ms IS NULL)) ORDER BY s.updated_at_ms DESC,s.id DESC LIMIT ?")
                    .bind::<Text,_>(&scope.0).bind::<Text,_>(pattern)
                    .bind::<Nullable<BigInt>,_>(filters.from_ms).bind::<Nullable<BigInt>,_>(filters.from_ms).bind::<Nullable<BigInt>,_>(filters.to_ms).bind::<Nullable<BigInt>,_>(filters.to_ms)
                    .bind::<Nullable<Text>,_>(filters.script_name.clone()).bind::<Nullable<Text>,_>(filters.script_name.clone()).bind::<Nullable<Text>,_>(filters.status.clone()).bind::<Nullable<Text>,_>(filters.status.clone())
                    .bind::<BigInt,_>(limit*10).load::<HitRow>(&mut conn)?
            } else {
                let literal=format!("\"{}\"",query.replace('"',"\"\""));
                sql_query("SELECT s.id AS session_id,s.title,s.updated_at_ms,bm25(chat_search_fts) AS score,snippet(chat_search_fts,0,'','', '…',16) AS snippet FROM chat_search_fts JOIN chat_search_documents d ON d.id=chat_search_fts.rowid JOIN sessions s ON s.id=d.session_id WHERE chat_search_fts MATCH ? AND s.user_id=? AND (? IS NULL OR s.updated_at_ms>=?) AND (? IS NULL OR s.updated_at_ms<=?) AND (? IS NULL OR EXISTS(SELECT 1 FROM simulation_runs r JOIN script_resources sr ON sr.id=r.script_resource_id WHERE r.session_id=s.id AND sr.name=? AND r.deleted_at_ms IS NULL)) AND (? IS NULL OR EXISTS(SELECT 1 FROM simulation_runs r WHERE r.session_id=s.id AND r.status=? AND r.deleted_at_ms IS NULL)) ORDER BY score ASC,s.updated_at_ms DESC,s.id DESC LIMIT ?")
                    .bind::<Text,_>(literal).bind::<Text,_>(&scope.0)
                    .bind::<Nullable<BigInt>,_>(filters.from_ms).bind::<Nullable<BigInt>,_>(filters.from_ms).bind::<Nullable<BigInt>,_>(filters.to_ms).bind::<Nullable<BigInt>,_>(filters.to_ms)
                    .bind::<Nullable<Text>,_>(filters.script_name.clone()).bind::<Nullable<Text>,_>(filters.script_name).bind::<Nullable<Text>,_>(filters.status.clone()).bind::<Nullable<Text>,_>(filters.status)
                    .bind::<BigInt,_>(limit*10).load::<HitRow>(&mut conn)?
            };
            let mut seen=std::collections::HashSet::new();
            Ok(rows.into_iter().filter(|r|seen.insert(r.session_id)).take(limit as usize).map(|r|HistoryHit{session_id:r.session_id.to_string(),title:r.title,updated_at_ms:r.updated_at_ms,score:r.score,snippet:r.snippet}).collect())
        }).await.map_err(RepoError::Join)?
    }
    pub async fn messages(
        &self,
        scope: &UserId,
        session_id: i32,
        before: Option<i32>,
        around: Option<i32>,
        limit: i64,
    ) -> RepoResult<Vec<Message>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(scope.0))
                .select(schema::sessions::id)
                .first::<i32>(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let limit = limit.clamp(1, 100);
            if before.is_some() && around.is_some() {
                return Err(RepoError::InvalidState(
                    "beforeId 与 aroundId 不能同时使用".into(),
                ));
            }
            let mut rows = if let Some(id) = around {
                schema::messages::table
                    .filter(schema::messages::session_id.eq(session_id))
                    .filter(schema::messages::id.le(id))
                    .order(schema::messages::id.desc())
                    .limit(limit / 2 + 1)
                    .load::<Message>(&mut c)?
            } else if let Some(id) = before {
                schema::messages::table
                    .filter(schema::messages::session_id.eq(session_id))
                    .filter(schema::messages::id.lt(id))
                    .order(schema::messages::id.desc())
                    .limit(limit)
                    .load::<Message>(&mut c)?
            } else {
                schema::messages::table
                    .filter(schema::messages::session_id.eq(session_id))
                    .order(schema::messages::id.desc())
                    .limit(limit)
                    .load::<Message>(&mut c)?
            };
            rows.reverse();
            if let Some(id) = around {
                let remaining = (limit - rows.len() as i64).max(0);
                let mut after = schema::messages::table
                    .filter(schema::messages::session_id.eq(session_id))
                    .filter(schema::messages::id.gt(id))
                    .order(schema::messages::id.asc())
                    .limit(remaining)
                    .load::<Message>(&mut c)?;
                rows.append(&mut after);
            }
            Ok(rows)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn rebuild(&self) -> RepoResult<()> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move||{let mut c=pool.get().map_err(RepoError::Pool)?;c.transaction::<_,RepoError,_>(|c|{diesel::delete(schema::chat_search_documents::table).execute(c)?;sql_query("INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT id,'session_title',id,title,updated_at_ms FROM sessions").execute(c)?;sql_query("INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT session_id,'message',id,content,created_at_ms FROM messages WHERE role IN ('user','assistant')").execute(c)?;sql_query("INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT session_id,'model_import',id,source_summary_json,created_at_ms FROM model_imports").execute(c)?;sql_query("INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT session_id,'script',id,name,created_at_ms FROM script_resources").execute(c)?;sql_query("INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT r.session_id,'run_summary',r.id,s.name||' '||r.status,COALESCE(r.finished_at_ms,r.started_at_ms,r.created_at_ms) FROM simulation_runs r JOIN script_resources s ON s.id=r.script_resource_id WHERE r.deleted_at_ms IS NULL").execute(c)?;Ok(())})}).await.map_err(RepoError::Join)?
    }
}
#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type=BigInt)]
    #[allow(dead_code)]
    value: i64,
}
fn escape_like(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
