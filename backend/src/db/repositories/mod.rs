mod analysis;
mod artifact;
mod comparison;
mod history;
mod message;
mod model_import;
mod result;
mod run;
mod script;
mod session;
mod storage_scope;
mod user;
mod version;

pub use analysis::AnalysisRepo;
pub use artifact::{ArtifactDraft, ArtifactRepo};
pub use comparison::ComparisonRepo;
pub use history::{HistoryHit, HistoryRepo, SearchFilters};
pub use message::MessageRepo;
pub use model_import::{ModelImportRecord, ModelImportRepo};
pub use result::{DeleteResultError, DeletedResult, LineageRow, ResultRepo, ResultViewRow};
pub use run::{
    CreateRun, CreatedRun, NewRunDataset, RunContext, RunDataset, RunModelFrame, RunRepo,
    RunRepoError,
};
pub use script::ScriptRepo;
pub use session::SessionRepo;
pub use storage_scope::{StorageLimits, StorageScopeRepo, StorageUsage};
pub use user::UserRepo;
pub use version::{
    DeleteResult, DeletionImpact, DraftDataFrameReadRow, DraftSnapshot, PublishResult,
    VersionDataFrameReadRow, VersionDataframeRow, VersionRepo, VersionRepoError, VersionRow,
    VersionSnapshot, WorkspaceRow, WorkspaceSnapshot,
};

#[derive(Debug)]
pub enum RepoError {
    Pool(diesel::r2d2::PoolError),
    Query(diesel::result::Error),
    Join(tokio::task::JoinError),
    NotFound,
    QuotaExceeded,
    Storage(String),
    VersionConflict,
    InvalidState(String),
    IdempotencyConflict,
}

impl std::fmt::Display for RepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoError::Pool(error) => write!(f, "连接池错误: {error}"),
            RepoError::Query(error) => write!(f, "查询错误: {error}"),
            RepoError::Join(error) => write!(f, "阻塞任务错误: {error}"),
            RepoError::NotFound => write!(f, "资源不存在"),
            RepoError::QuotaExceeded => write!(f, "存储配额不足"),
            RepoError::Storage(error) => write!(f, "存储错误: {error}"),
            RepoError::VersionConflict => write!(f, "模型版本基线已变化"),
            RepoError::InvalidState(error) => write!(f, "状态无效: {error}"),
            RepoError::IdempotencyConflict => write!(f, "幂等键对应的请求内容不一致"),
        }
    }
}

impl std::error::Error for RepoError {}

impl From<diesel::result::Error> for RepoError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Query(error)
    }
}

pub type RepoResult<T> = Result<T, RepoError>;

pub(super) struct StateTransition {
    pub status: &'static str,
    pub stage: String,
    pub error: Option<String>,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

impl StateTransition {
    pub fn running(stage: &str, now_ms: i64) -> Self {
        Self {
            status: "running",
            stage: stage.to_string(),
            error: None,
            started_at_ms: Some(now_ms),
            finished_at_ms: None,
        }
    }

    pub fn failed(stage: &str, error: String, now_ms: i64) -> Self {
        Self {
            status: "failed",
            stage: stage.to_string(),
            error: Some(error),
            started_at_ms: None,
            finished_at_ms: Some(now_ms),
        }
    }
}
