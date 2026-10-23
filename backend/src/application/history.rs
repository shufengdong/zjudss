use crate::{
    auth::CurrentUser,
    db::{
        models::Message,
        repositories::{HistoryHit, HistoryRepo, RepoError, SearchFilters},
    },
};
#[derive(Debug)]
pub struct HistoryError {
    pub code: &'static str,
    pub message: String,
}
impl HistoryError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for HistoryError {}
#[derive(Clone)]
pub struct HistoryService {
    repo: HistoryRepo,
}
impl HistoryService {
    pub fn new(repo: HistoryRepo) -> Self {
        Self { repo }
    }
    pub async fn search_sessions(
        &self,
        p: &CurrentUser,
        query: String,
        filters: SearchFilters,
        limit: i64,
    ) -> Result<Vec<HistoryHit>, HistoryError> {
        if query.trim().is_empty() {
            return Err(HistoryError::new("history_query_invalid", "搜索词不能为空"));
        }
        if query.chars().count() > 2 && !self.repo.fts_available().await {
            return Err(HistoryError::new(
                "history_search_unavailable",
                "SQLite FTS5 trigram 不可用",
            ));
        }
        self.repo
            .search(&p.id, query, filters, limit)
            .await
            .map_err(map)
    }
    pub async fn list_messages(
        &self,
        p: &CurrentUser,
        session: i32,
        before: Option<i32>,
        around: Option<i32>,
        limit: i64,
    ) -> Result<Vec<Message>, HistoryError> {
        self.repo
            .messages(&p.id, session, before, around, limit)
            .await
            .map_err(map)
    }
    pub async fn read_context(
        &self,
        p: &CurrentUser,
        session: i32,
        around: i32,
        radius: i64,
    ) -> Result<Vec<Message>, HistoryError> {
        self.list_messages(p, session, None, Some(around), radius.clamp(2, 50))
            .await
    }
    pub async fn rebuild_index(&self) -> Result<(), HistoryError> {
        self.repo.rebuild().await.map_err(map)
    }
}
fn map(e: RepoError) -> HistoryError {
    match e {
        RepoError::NotFound => HistoryError::new("not_found", "资源不存在"),
        RepoError::InvalidState(v) => HistoryError::new("history_page_invalid", v),
        v => HistoryError::new("history_error", v.to_string()),
    }
}
