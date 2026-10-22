//! 编排层：全局状态、事件总线与一次对话的编排。
//!
//! `AppState` 是 axum 的共享状态（连接池、仓储、事件总线、引擎适配器、限流信号量）；
//! `EventBus` 按 session 广播事件供 SSE 订阅；`ChatService` 编排一次 agent 对话。

pub mod artifacts;
pub mod chat;
mod events;
pub mod example_import;
pub mod ingestion;
pub mod model_patch;
pub mod scheduler;
pub mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::adapters::{SimEngine, resolve_data_dir};
use crate::agent::LlmConfig;
use crate::agent::knowledge::AgentKnowledge;
use crate::auth::AuthConfig;
use crate::catalog::SchemaCatalog;
use crate::db::DbPool;
use crate::db::repositories::{
    ArtifactRepo, IngestionRepo, MessageRepo, SessionRepo, StorageScopeRepo, TaskRepo,
    WorkspaceRepo,
};
use crate::engine_resources::{EngineKnowledge, EngineResourceSnapshot};
use crate::ingestion::ArchiveLimits;
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
pub use example_import::{ExampleImportError, ExampleImportService};
pub use ingestion::{AcceptedBatch, IngestionError, IngestionService, UploadBatch, UploadFile};
pub use model_patch::{ModelChange, ModelMutationError, ModelPatch, ModelPatchService};
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
    pub example_import_service: ExampleImportService,
    pub model_patch_service: ModelPatchService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
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
        let engine_snapshot = Arc::new(EngineResourceSnapshot {
            id: "embedded-empty".into(),
            contract_version: "embedded-test".into(),
            catalog_schema_version: 2,
            source_root: data_dir.clone(),
            engine_root: data_dir.clone(),
            catalog_json: serde_json::json!({}),
            functions_json: serde_json::json!({}),
            catalog_bytes: Default::default(),
            documents: Default::default(),
            knowledge: EngineKnowledge::from_documents(&[]),
            examples: vec![],
        });
        Self::new_with_resources(
            pool,
            engine,
            llm,
            data_dir,
            auth,
            scheduler,
            storage_limits,
            catalog,
            engine_snapshot,
            50 * 1024 * 1024,
            500 * 1024 * 1024,
            ArchiveLimits::default(),
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
        engine_snapshot: Arc<EngineResourceSnapshot>,
        upload_single_file_limit_bytes: u64,
        upload_batch_limit_bytes: u64,
        archive_limits: ArchiveLimits,
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
            archive_limits,
        );
        let example_import_service = ExampleImportService::new(
            engine_snapshot.clone(),
            artifact_service.clone(),
            workspace_service.clone(),
            upload_single_file_limit_bytes,
        );
        let model_patch_service = ModelPatchService::new(workspace_service.clone(), artifact_service.clone());
        let agent_knowledge = Arc::new(
            AgentKnowledge::load_embedded("v2.0.0")
                .expect("内置 agent knowledge 已在构建时校验"),
        );
        Self {
            pool,
            session_repo,
            message_repo,
            task_repo,
            artifact_service,
            workspace_service,
            ingestion_service,
            example_import_service,
            model_patch_service,
            engine_snapshot,
            agent_knowledge,
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
    pub fn chat(&self, principal: crate::auth::Principal) -> ChatService {
        ChatService::new(self, principal)
    }
}
