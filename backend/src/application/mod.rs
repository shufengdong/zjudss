//! 编排层：全局状态、事件总线与一次对话的编排。
//!
//! `AppState` 是 axum 的共享状态（连接池、仓储、事件总线、引擎适配器、限流信号量）；
//! `EventBus` 按 session 广播事件供 SSE 订阅；`ChatService` 编排一次 agent 对话。

pub mod artifacts;
pub mod chat;
mod events;
pub mod ingestion;
pub mod scheduler;
pub mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::adapters::{SimEngine, resolve_data_dir};
use crate::agent::LlmConfig;
use crate::auth::{AccessScope, AuthConfig};
use crate::catalog::SchemaCatalog;
use crate::db::DbPool;
use crate::db::repositories::{
    ArtifactRepo, IngestionRepo, MessageRepo, SessionRepo, StorageScopeRepo, TaskRepo,
    WorkspaceRepo,
};
use crate::storage::ArtifactStore;

pub use artifacts::{
    ArtifactError, ArtifactMetadata, ArtifactRef, ArtifactService, GcReport, StorageLimits,
    StorageUsage,
};
pub use chat::ChatService;
pub use events::{
    AppEvent, AttachmentFile, DatasetViewRef, EventBus, EventWriter, ModelAmbiguity, ModelScope,
    ModelVersionSnapshot, ModelVersionStatus, OperationKind, OperationRef, RecognizedTable,
    ResultColumn, ResultOutputRef, Stage, StageStatus, ToolStatus, UploadStatus, VisualizationKind,
    ms_to_iso, now_ms,
};
pub use ingestion::{AcceptedBatch, IngestionError, IngestionService, UploadBatch, UploadFile};
pub use scheduler::{AppError, RunScheduler, RunTicket, SchedulerError};
pub use workspace::{
    DataframeBindingDraft, SourceKind, VersionDraft, VersionInputDraft, VersionOrigin,
    WorkspaceError, WorkspaceService, canonical_json,
};

/// 全局共享状态（axum `State`）。
#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub session_repo: SessionRepo,
    pub message_repo: MessageRepo,
    pub task_repo: TaskRepo,
    pub artifact_service: ArtifactService,
    pub workspace_service: WorkspaceService,
    pub ingestion_service: IngestionService,
    pub events: EventBus,
    pub engine: Arc<dyn SimEngine>,
    pub llm: LlmConfig,
    pub auth: AuthConfig,
    pub scheduler: RunScheduler,
    pub semaphore: Arc<Semaphore>,
    /// 数据根目录（内核结果、上传附件落这里），由 `DATA_DIR` 环境变量或默认 `data` 决定。
    pub data_dir: PathBuf,
    pub upload_single_file_limit_bytes: u64,
    pub upload_batch_limit_bytes: u64,
}

impl AppState {
    pub fn new(pool: DbPool, engine: Arc<dyn SimEngine>, llm: LlmConfig) -> Self {
        Self::new_with_data_dir(pool, engine, llm, resolve_data_dir())
    }

    pub fn new_with_data_dir(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
    ) -> Self {
        Self::new_with_data_dir_and_auth(pool, engine, llm, data_dir, AuthConfig::instance())
    }

    pub fn new_with_data_dir_and_auth(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
        auth: AuthConfig,
    ) -> Self {
        Self::new_with_data_dir_auth_and_scheduler(
            pool,
            engine,
            llm,
            data_dir,
            auth,
            RunScheduler::new(8, 2, 32),
        )
    }

    pub fn new_with_data_dir_auth_and_scheduler(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
        auth: AuthConfig,
        scheduler: RunScheduler,
    ) -> Self {
        Self::new_with_storage_limits(
            pool,
            engine,
            llm,
            data_dir,
            auth,
            scheduler,
            StorageLimits::default(),
        )
    }

    pub fn new_with_storage_limits(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
        auth: AuthConfig,
        scheduler: RunScheduler,
        storage_limits: StorageLimits,
    ) -> Self {
        let catalog = Arc::new(SchemaCatalog::from_value(
            "embedded-test",
            &serde_json::json!({"schema_version":2,"frames":{"table":{"required":{"id":"str"}}}}),
        ).expect("内置最小 catalog 有效"));
        Self::new_with_resources(
            pool,
            engine,
            llm,
            data_dir,
            auth,
            scheduler,
            storage_limits,
            catalog,
            50 * 1024 * 1024,
            500 * 1024 * 1024,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_resources(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
        auth: AuthConfig,
        scheduler: RunScheduler,
        storage_limits: StorageLimits,
        catalog: Arc<SchemaCatalog>,
        upload_single_file_limit_bytes: u64,
        upload_batch_limit_bytes: u64,
    ) -> Self {
        let session_repo = SessionRepo::new(pool.clone());
        let message_repo = MessageRepo::new(pool.clone());
        let task_repo = TaskRepo::new(pool.clone());
        let events = EventBus::new();
        let artifact_service = ArtifactService::new(
            ArtifactStore::new(data_dir.clone()).expect("DATA_DIR 已在配置阶段验证为可写"),
            ArtifactRepo::new(pool.clone()),
            StorageScopeRepo::new(pool.clone(), storage_limits),
        );
        let workspace_service = WorkspaceService::new(
            WorkspaceRepo::new(pool.clone()),
            message_repo.clone(),
            events.clone(),
        );
        let ingestion_service = IngestionService::new(
            IngestionRepo::new(pool.clone()),
            artifact_service.clone(),
            workspace_service.clone(),
            catalog,
            message_repo.clone(),
            events.clone(),
        );
        Self {
            pool,
            session_repo,
            message_repo,
            task_repo,
            artifact_service,
            workspace_service,
            ingestion_service,
            events,
            engine,
            llm,
            auth,
            scheduler,
            semaphore: Arc::new(Semaphore::new(4)),
            data_dir,
            upload_single_file_limit_bytes,
            upload_batch_limit_bytes,
        }
    }

    /// 派生一次对话的编排服务（共享引擎/事件/仓储，克隆成本低）。
    pub fn chat(&self, scope: AccessScope) -> ChatService {
        ChatService::new(self, scope)
    }
}
