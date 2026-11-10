mod analysis;
mod artifact;
mod artifact_refs;
mod case_switch;
mod chart_view;
mod comparison;
mod data_resource;
mod engine_resource_snapshot;
mod history;
mod managed_workflow;
mod message;
mod resource_lineage;
mod result;
mod run;
mod script;
mod session;
mod storage_scope;
mod study_configuration;
mod upload;
mod upload_confirmation;
mod user;
mod version;
mod workflow_input;

use diesel::RunQueryDsl;
use diesel::sql_types::Integer;
use diesel::sqlite::SqliteConnection;

pub use analysis::AnalysisRepo;
pub use artifact::{ArtifactDraft, ArtifactRepo};
pub use chart_view::{
    ChartViewDraft, ChartViewRecord, ChartViewRepo, ChartViewSummaryRow, DeletedChartView,
    PublishedChartView,
};
pub use comparison::ComparisonRepo;
pub use data_resource::{
    DataResourceRepo, DeletedResource, DeletionImpactRow, DeletionPreviewInput,
    PromoteDatasetInput, ResourcePageRow, StructuredPreviewConfirmation, StructuredPreviewInput,
};
pub use engine_resource_snapshot::EngineResourceSnapshotRepo;
pub use history::{HistoryHit, HistoryRepo, SearchFilters};
pub(crate) use managed_workflow::recover_interrupted_workflows;
pub use managed_workflow::{
    ConfirmStartOutcome, ManagedWorkflowAggregate, ManagedWorkflowRepo, ManagedWorkflowRepoError,
    ManagedWorkflowStartCommand, NewWorkflowCandidate, PendingRunDataset, WorkflowAttemptUpdate,
    WorkflowStructOverlayMaterial,
};
pub use message::MessageRepo;
pub use result::{DeleteResultError, DeletedResult, LineageRow, ResultRepo, ResultViewRow};
pub(crate) use run::RunArtifactOutputs;
pub use run::{
    CreateRun, CreatedRun, NewRunDataset, RunContext, RunDataset, RunDeletionImpact, RunModelFrame,
    RunRepo, RunRepoError,
};
pub use script::ScriptRepo;
pub use session::{SessionDeletionImpact, SessionRepo};
pub use storage_scope::{StorageLimits, StorageScopeRepo, StorageUsage};
pub use study_configuration::{
    NewStudyConfiguration, StudyConfigurationAggregate, StudyConfigurationRepo,
};
pub use upload::{
    NewUploadCandidate, NewUploadFile, RecoverableUpload, SettledUpload, UploadBatchRecordSet,
    UploadRepo,
};
pub use upload_confirmation::{
    DraftApplicationMode, FinishedUploadBatch, PublishChildResult, ResourcePublishInput,
    UploadConfirmationRepo,
};
pub use user::UserRepo;
pub use version::{
    DeletionImpact, DraftDataFrameReadRow, DraftSnapshot, PublishCommand, PublishResult,
    VersionDataFrameReadRow, VersionDataframeRow, VersionRepo, VersionRepoError, VersionRow,
    VersionSnapshot, WorkspaceRow, WorkspaceSnapshot,
};
pub(crate) use workflow_input::resolve_frozen_struct_input_on;
pub(crate) use workflow_input::{CommittedOverlayInput, commit_overlay_inputs_on};
pub use workflow_input::{
    OverlayArtifactSource, StructOverlayRequest, WorkflowInputRepo, WorkflowOverlayInput,
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

pub(super) fn last_insert_rowid(conn: &mut SqliteConnection) -> RepoResult<i32> {
    diesel::select(diesel::dsl::sql::<Integer>("last_insert_rowid()"))
        .get_result(conn)
        .map_err(RepoError::from)
}

pub(super) struct StateTransition {
    pub status: &'static str,
    pub stage: String,
    pub error: Option<String>,
    pub exit_code: Option<i32>,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

impl StateTransition {
    pub fn running(stage: &str, now_ms: i64) -> Self {
        Self {
            status: "running",
            stage: stage.to_string(),
            error: None,
            exit_code: None,
            started_at_ms: Some(now_ms),
            finished_at_ms: None,
        }
    }

    pub fn failed(stage: &str, error: String, now_ms: i64) -> Self {
        Self {
            status: "failed",
            stage: stage.to_string(),
            error: Some(error),
            exit_code: None,
            started_at_ms: None,
            finished_at_ms: Some(now_ms),
        }
    }

    pub fn failed_with_exit_code(
        stage: &str,
        error: String,
        exit_code: Option<i32>,
        now_ms: i64,
    ) -> Self {
        Self {
            exit_code,
            ..Self::failed(stage, error, now_ms)
        }
    }
}

#[cfg(test)]
mod tests {
    use diesel::Connection;
    use diesel::connection::SimpleConnection;
    use diesel::sqlite::SqliteConnection;

    use super::last_insert_rowid;

    #[test]
    fn last_insert_rowid_is_connection_local_and_monotonic() {
        let mut conn = SqliteConnection::establish(":memory:").unwrap();
        conn.batch_execute("CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT)")
            .unwrap();

        conn.batch_execute("INSERT INTO items DEFAULT VALUES")
            .unwrap();
        assert_eq!(last_insert_rowid(&mut conn).unwrap(), 1);

        conn.batch_execute("INSERT INTO items DEFAULT VALUES")
            .unwrap();
        assert_eq!(last_insert_rowid(&mut conn).unwrap(), 2);
    }
}
