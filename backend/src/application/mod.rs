//! 编排层：全局状态、事件总线与一次对话的编排。
//!
//! `AppState` 是 axum 的共享状态（连接池、仓储、事件总线、引擎适配器、限流信号量）；
//! `EventBus` 按 session 广播事件供 SSE 订阅；`ChatService` 编排一次 agent 对话。

pub mod analysis;
pub mod artifacts;
pub mod chat;
pub mod comparisons;
pub mod datasets;
mod events;
pub mod example_import;
pub mod history;
pub mod ingestion;
pub mod model_patch;
pub mod scheduler;
pub mod simulations;
pub mod visualization;
pub mod workspace;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::adapters::{SimEngine, resolve_data_dir};
use crate::agent::knowledge::AgentKnowledge;
use crate::agent::{AgentFactory, DeepSeekAgentFactory, LlmConfig};
use crate::auth::AuthConfig;
use crate::catalog::SchemaCatalog;
use crate::db::DbPool;
use crate::db::repositories::{
    AnalysisRepo, ArtifactRepo, ComparisonRepo, DatasetRepo, HistoryRepo, IngestionRepo,
    MessageRepo, SessionRepo, SimulationRepo, StorageScopeRepo, TaskRepo, WorkspaceRepo,
};
use crate::engine_resources::{EngineKnowledge, EngineResourceSnapshot};
use crate::ingestion::ArchiveLimits;
use crate::storage::ArtifactStore;

pub use analysis::{
    AnalysisError, AnalysisInput, AnalysisOutcome, AnalysisRequest, AnalysisService,
};
pub use artifacts::{
    ArtifactError, ArtifactMetadata, ArtifactRef, ArtifactService, GcReport, StorageLimits,
    StorageUsage, StorageUsageSummary,
};
pub use chat::ChatService;
pub use comparisons::{ComparisonError, ComparisonService};
pub use datasets::{DatasetError, DatasetService};
pub use events::{
    AppEvent, AttachmentFile, DatasetViewRef, EventBus, EventWriter, MESSAGE_TYPES, ModelAmbiguity,
    ModelScope, ModelVersionSnapshot, ModelVersionStatus, OperationKind, OperationRef,
    RecognizedTable, ResultColumn, ResultOutputRef, Stage, StageStatus, ToolStatus, UploadStatus,
    VisualizationKind, ms_to_iso, now_ms,
};
pub use example_import::{ExampleImportError, ExampleImportService};
pub use history::{HistoryError, HistoryService};
pub use ingestion::{AcceptedBatch, IngestionError, IngestionService, UploadBatch, UploadFile};
pub use model_patch::{ModelChange, ModelMutationError, ModelPatch, ModelPatchService};
pub use scheduler::{AppError, RunScheduler, RunTicket, SchedulerError};
pub use simulations::{RunStudyRequest, SimulationError, SimulationOutcome, SimulationService};
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
    pub dataset_service: DatasetService,
    pub simulation_service: SimulationService,
    pub analysis_service: AnalysisService,
    pub comparison_service: ComparisonService,
    pub history_service: HistoryService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub events: EventBus,
    pub engine: Arc<dyn SimEngine>,
    pub agent_factory: Arc<dyn AgentFactory>,
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

    pub fn new_with_agent_factory(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        agent_factory: Arc<dyn AgentFactory>,
    ) -> Self {
        Self::new_with_data_dir_and_agent_factory(
            pool,
            engine,
            agent_factory,
            resolve_data_dir(),
            AuthConfig::instance(),
            RunScheduler::new(8, 2, 32),
            StorageLimits::default(),
        )
    }

    #[cfg(feature = "e2e-test")]
    pub fn new_for_e2e(
        pool: DbPool,
        data_dir: PathBuf,
        agent_factory: Arc<dyn AgentFactory>,
        engine: Arc<dyn SimEngine>,
        auth: AuthConfig,
    ) -> Self {
        use sha2::{Digest, Sha256};

        use crate::engine_resources::{EngineExample, ExampleSourceFile};

        let engine_root = data_dir
            .parent()
            .unwrap_or(&data_dir)
            .join("engine-fixture");
        let relative = "sparrowzz/dss/examples/e2e-demo.rss";
        let source = engine_root.join(relative);
        std::fs::create_dir_all(source.parent().expect("example parent"))
            .expect("create e2e example directory");
        let source_bytes = b"result = 1\n";
        std::fs::write(&source, source_bytes).expect("write e2e example");
        // macOS 的临时目录常以 /var 暴露、canonicalize 后变为 /private/var；
        // 快照根必须规范化，否则案例导入的越界检查会误判合法 fixture。
        let engine_root = engine_root
            .canonicalize()
            .expect("canonicalize e2e engine root");
        let source_hash = format!("{:x}", Sha256::digest(source_bytes));
        let docs = vec![(
            "docs/e2e-guide.md".to_string(),
            "# E2E guide\nUse deterministic scenarios.".to_string(),
        )];
        let catalog_value = serde_json::json!({
            "schema_version": 2,
            "frames": {"bus": {"required": {"id": "str"}}}
        });
        let catalog = Arc::new(
            SchemaCatalog::from_value("e2e-contract", &catalog_value).expect("e2e catalog valid"),
        );
        let engine_snapshot = Arc::new(EngineResourceSnapshot {
            id: "e2e-snapshot".into(),
            contract_version: "e2e-contract".into(),
            catalog_schema_version: 2,
            source_root: engine_root.join("sparrowzz/dss"),
            engine_root: engine_root.clone(),
            catalog_json: catalog_value,
            functions_json: serde_json::json!({"schema_version": 2, "frame_functions": []}),
            catalog_bytes: Default::default(),
            documents: std::collections::BTreeMap::from([(
                docs[0].0.clone(),
                docs[0].1.as_bytes().to_vec(),
            )]),
            knowledge: EngineKnowledge::from_documents(&docs),
            examples: vec![EngineExample {
                id: "e2e-demo".into(),
                title: "E2E demo".into(),
                source_kind: "rss".into(),
                entry_path: "examples/e2e-demo.rss".into(),
                study_type: "power_flow".into(),
                solver: None,
                required_features: vec![],
                file_count: 1,
                size_bytes: source_bytes.len() as u64,
                tree_hash: source_hash.clone(),
                importable: true,
                diagnostics: vec![],
                files: vec![ExampleSourceFile {
                    virtual_path: relative.into(),
                    source_path: relative.into(),
                    sha256: source_hash,
                }],
            }],
        });
        Self::new_with_resources_and_agent_factory(
            pool,
            engine,
            agent_factory,
            data_dir,
            auth,
            RunScheduler::new(4, 2, 16),
            StorageLimits {
                artifact_limit: 100,
                byte_limit: 64 * 1024 * 1024,
                warning_ratio: 0.8,
            },
            catalog,
            engine_snapshot,
            2 * 1024 * 1024,
            8 * 1024 * 1024,
            ArchiveLimits {
                max_entries: 100,
                max_expanded_bytes: 16 * 1024 * 1024,
                max_compression_ratio: 20,
                max_file_bytes: 2 * 1024 * 1024,
            },
        )
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
        Self::new_with_resources_and_agent_factory(
            pool,
            engine,
            Arc::new(DeepSeekAgentFactory::new(llm)),
            data_dir,
            auth,
            scheduler,
            storage_limits,
            catalog,
            engine_snapshot,
            upload_single_file_limit_bytes,
            upload_batch_limit_bytes,
            archive_limits,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_resources_and_agent_factory(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        agent_factory: Arc<dyn AgentFactory>,
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
        let model_patch_service =
            ModelPatchService::new(workspace_service.clone(), artifact_service.clone());
        let dataset_service =
            DatasetService::new(DatasetRepo::new(pool.clone()), artifact_service.clone());
        let agent_knowledge = Arc::new(
            AgentKnowledge::load_embedded("v2.0.0").expect("内置 agent knowledge 已在构建时校验"),
        );
        let simulation_service = SimulationService::new(
            SimulationRepo::new(pool.clone()),
            workspace_service.clone(),
            artifact_service.clone(),
            message_repo.clone(),
            events.clone(),
            engine.clone(),
            agent_knowledge.clone(),
            data_dir.clone(),
        );
        let analysis_service = AnalysisService::new(
            AnalysisRepo::new(pool.clone()),
            DatasetRepo::new(pool.clone()),
            artifact_service.clone(),
            message_repo.clone(),
            events.clone(),
            engine.clone(),
            agent_knowledge.clone(),
            data_dir.clone(),
        );
        let comparison_service = ComparisonService::new(
            ComparisonRepo::new(pool.clone()),
            SimulationRepo::new(pool.clone()),
            DatasetRepo::new(pool.clone()),
            analysis_service.clone(),
            message_repo.clone(),
            events.clone(),
        );
        let history_service = HistoryService::new(
            HistoryRepo::new(pool.clone()),
            SimulationRepo::new(pool.clone()),
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
            dataset_service,
            simulation_service,
            analysis_service,
            comparison_service,
            history_service,
            engine_snapshot,
            agent_knowledge,
            events,
            engine,
            agent_factory,
            auth,
            scheduler,
            semaphore: Arc::new(Semaphore::new(4)),
            data_dir,
            upload_single_file_limit_bytes,
            upload_batch_limit_bytes,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_data_dir_and_agent_factory(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        agent_factory: Arc<dyn AgentFactory>,
        data_dir: PathBuf,
        auth: AuthConfig,
        scheduler: RunScheduler,
        storage_limits: StorageLimits,
    ) -> Self {
        let catalog = Arc::new(
            SchemaCatalog::from_value(
                "embedded-test",
                &serde_json::json!({"schema_version":2,"frames":{"table":{"required":{"id":"str"}}}}),
            )
            .expect("内置最小 catalog 有效"),
        );
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
        Self::new_with_resources_and_agent_factory(
            pool,
            engine,
            agent_factory,
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

    /// 派生一次对话的编排服务（共享引擎/事件/仓储，克隆成本低）。
    pub fn chat(&self, principal: crate::auth::Principal) -> ChatService {
        ChatService::new(self, principal)
    }
}
