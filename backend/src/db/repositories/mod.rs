mod analysis;
mod artifact;
mod comparison;
mod dataset;
mod history;
mod ingestion;
mod message;
mod model_import;
mod script;
mod session;
mod simulation;
mod storage_scope;
mod task;
mod user;
mod workspace;

pub use analysis::AnalysisRepo;
pub use artifact::{ArtifactDraft, ArtifactRepo};
pub use comparison::ComparisonRepo;
pub use dataset::DatasetRepo;
pub use history::{HistoryHit, HistoryRepo, SearchFilters};
pub use ingestion::{IngestionItemDraft, IngestionRecord, IngestionRepo};
pub use message::MessageRepo;
pub use model_import::{ModelImportRecord, ModelImportRepo};
pub use script::ScriptRepo;
pub use session::SessionRepo;
pub use simulation::SimulationRepo;
pub use storage_scope::{StorageLimits, StorageScopeRepo, StorageUsage};
pub use task::TaskRepo;
pub use user::UserRepo;
pub use workspace::{
    BindingDraft, PreparedVersion, VersionArtifactDraft, WorkspaceRecord, WorkspaceRepo,
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
