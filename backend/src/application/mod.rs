//! 编排层：全局状态、事件总线与一次对话的编排。
//!
//! `AppState` 是 axum 的共享状态（连接池、仓储、事件总线、引擎适配器、限流信号量）；
//! `EventBus` 按 session 广播事件供 SSE 订阅；`ChatService` 编排一次 agent 对话。

pub mod analysis;
pub mod artifacts;
pub mod case_import;
pub mod charting;
pub mod chat;
pub mod comparisons;
mod data_mapping;
pub mod data_resources;
pub(crate) mod dataframe_reader;
pub mod datasets;
pub mod diagnostics;
mod events;
pub mod history;
pub mod managed_rss;
pub mod managed_workflows;
pub mod model_dataframes;
pub mod model_patch;
pub mod runs;
pub mod scheduler;
pub mod script_contract;
pub mod scripts;
mod study_configurations;
pub mod study_planning;
pub mod uploads;
pub mod versions;
pub mod workflow_worker;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::adapters::SimEngine;
use crate::agent::knowledge::AgentKnowledge;
use crate::agent::{
    AgentFactory, FailureAnalyzer, LlmFailureAnalyzer, LlmScriptAuthor, ScriptAuthor,
};
use crate::auth::AuthService;
use crate::catalog::SchemaCatalog;
use crate::db::DbPool;
use crate::db::repositories::{
    AnalysisRepo, ArtifactRepo, ChartViewRepo, ComparisonRepo, DataResourceRepo, HistoryRepo,
    ManagedWorkflowRepo, MessageRepo, ResultRepo, RunRepo, ScriptRepo, SessionRepo,
    StorageScopeRepo, StudyConfigurationRepo, UploadRepo, VersionRepo,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::ingestion::ArchiveLimits;
use crate::storage::ArtifactStore;

pub use analysis::{
    AnalysisError, AnalysisInput, AnalysisInputSource, AnalysisOutcome, AnalysisRequest,
    AnalysisService,
};
pub use artifacts::{
    ArtifactError, ArtifactMetadata, ArtifactRef, ArtifactService, GcReport, StorageLimits,
    StorageUsage, StorageUsageSummary,
};
pub use case_import::{CaseImportError, CaseImportService};
pub use charting::ChartService;
pub use chat::ChatService;
pub use comparisons::{ComparisonError, ComparisonService};
pub use data_mapping::{ColumnMapping, CreateMappedResourceRequest, DataMappingService};
pub use data_resources::{
    DataResourceDeletionPreview, DataResourceDownload, DataResourceError, DataResourcePage,
    DataResourceRows, DataResourceService, DataResourceView, DeletedDataResource,
    ReplacementAction, StructuredReplacementPreview, UploadConfirmation,
};
pub use datasets::{DatasetError, DatasetService};
pub use events::{
    AppEvent, AttachmentFile, DatasetViewRef, EventBus, EventDiagnostic, EventWriter,
    MESSAGE_TYPES, ModelAmbiguity, ModelScope, ModelVersionSnapshot, ModelVersionStatus,
    OperationKind, OperationRef, RecognizedTable, ResultColumn, ResultOutputRef, Stage,
    StageStatus, StudyConfigCard, ToolStatus, UploadBatchCard, UploadCandidateCard, UploadStatus,
    VisualizationKind, WorkflowAttemptCard, WorkflowProgressCard, ms_to_iso, now_ms,
};
pub use history::{HistoryError, HistoryService};
pub use managed_workflows::{
    ConfirmRetryRequest, ConfirmStudyRequest, ManagedWorkflowService, ManagedWorkflowView,
    RetryPreviewView, RetryStudyRequest, WorkflowAttemptSummary, WorkflowError,
};
pub use model_dataframes::{
    DraftReadExpectation, ModelDataFrameQuery, ModelDataFrameQueryResult, ModelDataFrameReadError,
    ModelDataFrameReadService, ModelReadScope,
};
pub use model_patch::{ModelChange, ModelMutationError, ModelPatch, ModelPatchService};
pub use runs::{
    CreateRunRequest, RunDeletionPreview, RunError, RunService, RunSubmission, RunSummary,
};
pub use scheduler::{
    AppError, RunScheduler, RunTicket, SchedulerError, SessionCancellationRegistry,
};
pub use scripts::{
    CreateScriptRequest, ScriptDetail, ScriptError, ScriptService, ScriptServiceDependencies,
    ScriptSource, ScriptSummary,
};
pub use study_configurations::*;
pub use study_planning::StudyPlanningService;
pub use uploads::{
    CandidateDecision, CandidatePurpose, ChildPublishStatus, ParseSettings, UploadBatchError,
    UploadBatchService, UploadBatchStatus, UploadBatchView, UploadCandidateView, UploadFile,
    UploadSourceKind,
};
pub use versions::{
    ActivateVersionRequest, PublishDraftRequest, PublishedVersion, VersionDeletionPreview,
    VersionError, VersionService, default_version_description, normalize_version_description,
};
pub fn canonical_json(value: &serde_json::Value) -> String {
    serde_json::to_string(value).expect("JSON value is serializable")
}

/// 全局共享状态（axum `State`）。
#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub session_repo: SessionRepo,
    pub message_repo: MessageRepo,
    pub artifact_service: ArtifactService,
    pub version_service: VersionService,
    pub upload_batch_service: UploadBatchService,
    pub data_resource_service: DataResourceService,
    pub data_mapping_service: DataMappingService,
    pub model_dataframe_read_service: ModelDataFrameReadService,
    pub script_service: ScriptService,
    pub run_service: RunService,
    pub case_import_service: CaseImportService,
    pub model_patch_service: ModelPatchService,
    pub dataset_service: DatasetService,
    pub chart_service: ChartService,
    pub analysis_service: AnalysisService,
    pub comparison_service: ComparisonService,
    pub history_service: HistoryService,
    pub study_planning_service: StudyPlanningService,
    pub study_configuration_service: StudyConfigurationService,
    pub managed_workflow_service: ManagedWorkflowService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub events: EventBus,
    pub engine: Arc<dyn SimEngine>,
    pub agent_factory: Arc<dyn AgentFactory>,
    pub script_author: Arc<dyn ScriptAuthor>,
    pub failure_analyzer: Arc<dyn FailureAnalyzer>,
    pub auth: AuthService,
    pub scheduler: RunScheduler,
    pub session_cancellations: SessionCancellationRegistry,
    pub semaphore: Arc<Semaphore>,
    /// 数据根目录（内核结果、上传附件落这里），由 `DATA_DIR` 环境变量或默认 `data` 决定。
    pub data_dir: PathBuf,
    pub upload_single_file_limit_bytes: u64,
    pub upload_batch_limit_bytes: u64,
}

#[cfg(feature = "e2e-test")]
pub struct E2eAppOptions {
    pub script_author: Option<Arc<dyn ScriptAuthor>>,
    pub failure_analyzer: Option<Arc<dyn FailureAnalyzer>>,
    pub managed_workflow_auto_start: bool,
    pub full_engine_catalog: bool,
}

pub struct AppStateDependencies {
    pub pool: DbPool,
    pub engine: Arc<dyn SimEngine>,
    pub agent_factory: Arc<dyn AgentFactory>,
    pub auth: AuthService,
    pub scheduler: RunScheduler,
}

pub struct AppStateOptions {
    pub data_dir: PathBuf,
    pub storage_limits: StorageLimits,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub upload_single_file_limit_bytes: u64,
    pub upload_batch_limit_bytes: u64,
    pub upload_staging_ttl_hours: u64,
    pub script_repair_max_attempts: usize,
    pub agent_model_retry_max: usize,
    pub managed_engine_timeout: std::time::Duration,
    pub managed_result_max_file_bytes: u64,
    pub script_author_override: Option<Arc<dyn ScriptAuthor>>,
    pub failure_analyzer_override: Option<Arc<dyn FailureAnalyzer>>,
    pub managed_workflow_auto_start: bool,
    pub archive_limits: ArchiveLimits,
}

impl AppState {
    #[cfg(feature = "e2e-test")]
    pub fn new_for_e2e(
        pool: DbPool,
        data_dir: PathBuf,
        agent_factory: Arc<dyn AgentFactory>,
        engine: Arc<dyn SimEngine>,
        auth: AuthService,
        options: E2eAppOptions,
    ) -> Self {
        use sha2::{Digest, Sha256};

        use crate::engine_resources::EngineResourceRegistry;

        let engine_root = data_dir
            .parent()
            .unwrap_or(&data_dir)
            .join("engine-fixture");
        let bundle_path = "E2E/Demo/E2E demo.rssbundle";
        let bundle_relative = format!("dss/rssbundle/{bundle_path}");
        let bundle = engine_root.join(&bundle_relative);
        std::fs::create_dir_all(bundle.join("dss")).expect("create e2e example directory");
        let main_bytes = b"model = import_dss(\"dss/model.dss\");\n";
        let managed_bytes = b"[result_voltage] = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);\n";
        let source_bytes = b"new circuit.e2e basekv=10\n";
        std::fs::write(bundle.join("main.rss"), main_bytes).expect("write e2e main");
        std::fs::write(bundle.join("managed.rss"), managed_bytes).expect("write e2e managed");
        std::fs::write(bundle.join("dss/model.dss"), source_bytes).expect("write e2e model");
        let main_hash = hex::encode(Sha256::digest(main_bytes));
        let managed_hash = hex::encode(Sha256::digest(managed_bytes));
        let source_hash = hex::encode(Sha256::digest(source_bytes));
        let manifest = serde_json::json!({
            "schema_version": 2,
            "artifact": "mems.dss-rss-bundle",
            "generator": "zjudss-e2e/1",
            "psa_contract_version": "0.1.0",
            "entry": "main.rss",
            "study": {
                "kind": "static_power_flow",
                "compatibility": "opendss-native",
                "solver": "newton",
                "results": ["voltage"]
            },
            "source": {
                "entry": "dss/model.dss",
                "root": "dss",
                "tree_sha256": source_hash
            },
            "projection": {
                "source_function": "ds_newton_raphson_pf",
                "profile": "dss_static_newton_v1",
                "projector": "project_dss_static_model",
                "target_function": "ds_newton_raphson_pf",
                "managed_entry": "managed.rss",
                "status": "enabled",
                "stable_diagnostic": "PSA.DSS.PROJECTION.INVALID",
                "results": [{"source":"voltage","target":"bus_phase","binding":"result_voltage"}]
            },
            "files": [
                {"path": "dss/model.dss", "role": "dss-entry", "sha256": source_hash},
                {"path": "main.rss", "role": "main-rss", "sha256": main_hash},
                {"path": "managed.rss", "role": "managed-rss", "sha256": managed_hash}
            ],
            "inputs": []
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("serialize e2e manifest");
        std::fs::write(bundle.join("manifest.json"), &manifest_bytes).expect("write e2e manifest");
        let invalid_relative = "dss/rssbundle/E2E/Demo/Broken.rssbundle";
        let invalid_bytes = b"{";
        std::fs::create_dir_all(engine_root.join(invalid_relative))
            .expect("create invalid e2e example directory");
        std::fs::write(
            engine_root.join(invalid_relative).join("manifest.json"),
            invalid_bytes,
        )
        .expect("write invalid e2e manifest");
        let catalog_dir = engine_root.join("dss/docs/catalog");
        std::fs::create_dir_all(&catalog_dir).expect("create e2e catalog directory");
        let dataframes =
            include_bytes!("../../tests/fixtures/engine_resources/catalog/dataframes.json");
        let functions =
            include_bytes!("../../tests/fixtures/engine_resources/catalog/functions.json");
        let study_facades =
            include_bytes!("../../tests/fixtures/engine_resources/catalog/study_facades.json");
        let case_projections =
            include_bytes!("../../tests/fixtures/engine_resources/catalog/case_projections.json");
        std::fs::write(catalog_dir.join("dataframes.json"), dataframes)
            .expect("write e2e dataframes catalog");
        std::fs::write(catalog_dir.join("functions.json"), functions)
            .expect("write e2e functions catalog");
        std::fs::write(catalog_dir.join("study_facades.json"), study_facades)
            .expect("write e2e study facades catalog");
        std::fs::write(catalog_dir.join("case_projections.json"), case_projections)
            .expect("write e2e case projections catalog");
        std::fs::write(
            engine_root.join("dss/docs/e2e-guide.md"),
            b"# E2E guide\nUse deterministic scenarios.",
        )
        .expect("write e2e guide");
        // macOS 的临时目录常以 /var 暴露、canonicalize 后变为 /private/var；
        // 快照根必须规范化，否则案例导入的越界检查会误判合法 fixture。
        let engine_root = engine_root
            .canonicalize()
            .expect("canonicalize e2e engine root");
        let loaded_snapshot =
            EngineResourceRegistry::load(&engine_root, &std::collections::HashSet::new())
                .expect("load complete e2e engine resources");
        let engine_snapshot = if options.full_engine_catalog {
            loaded_snapshot
        } else {
            EngineResourceSnapshot::minimal("e2e-snapshot", "e2e-contract", engine_root.join("dss"))
        };
        let catalog_value = engine_snapshot.catalog_json.clone();
        SchemaCatalog::from_value("e2e-contract", &catalog_value).expect("e2e catalog valid");
        let engine_snapshot = Arc::new(engine_snapshot);
        Self::assemble(
            AppStateDependencies {
                pool,
                engine,
                agent_factory,
                auth,
                scheduler: RunScheduler::new(4, 2, 16),
            },
            AppStateOptions {
                data_dir,
                storage_limits: StorageLimits {
                    artifact_limit: 100,
                    byte_limit: 64 * 1024 * 1024,
                    warning_ratio: 0.8,
                },
                engine_snapshot,
                upload_single_file_limit_bytes: 2 * 1024 * 1024,
                upload_batch_limit_bytes: 8 * 1024 * 1024,
                upload_staging_ttl_hours: 24,
                script_repair_max_attempts: 3,
                agent_model_retry_max: 2,
                managed_engine_timeout: std::time::Duration::from_secs(120),
                managed_result_max_file_bytes: 512 * 1024 * 1024,
                script_author_override: options.script_author,
                failure_analyzer_override: options.failure_analyzer,
                managed_workflow_auto_start: options.managed_workflow_auto_start,
                archive_limits: ArchiveLimits {
                    max_entries: 100,
                    max_expanded_bytes: 16 * 1024 * 1024,
                    max_compression_ratio: 20,
                    max_file_bytes: 2 * 1024 * 1024,
                },
            },
        )
    }

    pub fn assemble(dependencies: AppStateDependencies, options: AppStateOptions) -> Self {
        let AppStateDependencies {
            pool,
            engine,
            agent_factory,
            auth,
            scheduler,
        } = dependencies;
        let AppStateOptions {
            data_dir,
            storage_limits,
            engine_snapshot,
            upload_single_file_limit_bytes,
            upload_batch_limit_bytes,
            upload_staging_ttl_hours,
            script_repair_max_attempts,
            agent_model_retry_max,
            managed_engine_timeout,
            managed_result_max_file_bytes,
            script_author_override,
            failure_analyzer_override,
            managed_workflow_auto_start,
            archive_limits,
        } = options;
        let session_repo = SessionRepo::new(pool.clone());
        let script_author: Arc<dyn ScriptAuthor> = script_author_override.unwrap_or_else(|| {
            Arc::new(LlmScriptAuthor::from_factory(
                agent_factory.clone(),
                agent_model_retry_max,
            ))
        });
        let failure_analyzer: Arc<dyn FailureAnalyzer> =
            failure_analyzer_override.unwrap_or_else(|| {
                Arc::new(LlmFailureAnalyzer::from_factory(
                    agent_factory.clone(),
                    agent_model_retry_max,
                ))
            });
        let message_repo = MessageRepo::new(pool.clone());
        let events = EventBus::new();
        let session_cancellations = SessionCancellationRegistry::default();
        let artifact_service = ArtifactService::new(
            ArtifactStore::new(data_dir.clone()).expect("DATA_DIR 已在配置阶段验证为可写"),
            ArtifactRepo::new(pool.clone()),
            StorageScopeRepo::new(pool.clone(), storage_limits),
        );
        let version_service =
            VersionService::new(VersionRepo::new(pool.clone()), artifact_service.clone());
        let model_dataframe_read_service = ModelDataFrameReadService::new(
            VersionRepo::new(pool.clone()),
            artifact_service.clone(),
        );
        let upload_batch_service =
            UploadBatchService::new(uploads::UploadBatchServiceDependencies {
                repo: UploadRepo::new(pool.clone()),
                confirmation_repo: DataResourceRepo::new(pool.clone()),
                artifacts: artifact_service.clone(),
                engine: engine.clone(),
                snapshot: engine_snapshot.clone(),
                data_dir: data_dir.clone(),
                ttl_hours: upload_staging_ttl_hours,
                archive_limits,
            });
        let data_resource_service = DataResourceService::new(
            DataResourceRepo::new(pool.clone()),
            UploadRepo::new(pool.clone()),
            upload_batch_service.clone(),
            artifact_service.clone(),
            engine_snapshot.clone(),
            data_dir.clone(),
        );
        let data_mapping_service = DataMappingService::new(
            data_resource_service.clone(),
            upload_batch_service.clone(),
            UploadRepo::new(pool.clone()),
        );
        let script_service = ScriptService::new(scripts::ScriptServiceDependencies {
            repo: ScriptRepo::new(pool.clone()),
            artifacts: artifact_service.clone(),
            engine: engine.clone(),
            data_dir: data_dir.clone(),
            messages: message_repo.clone(),
            events: events.clone(),
            versions: VersionRepo::new(pool.clone()),
            engine_snapshot_id: engine_snapshot.id.clone(),
        });
        let run_service = RunService::new(runs::RunServiceDependencies {
            repo: RunRepo::new(pool.clone()),
            scripts: ScriptRepo::new(pool.clone()),
            artifacts: artifact_service.clone(),
            data_resources: data_resource_service.clone(),
            engine: engine.clone(),
            scheduler: scheduler.clone(),
            messages: message_repo.clone(),
            events: events.clone(),
            session_cancellations: session_cancellations.clone(),
            data_dir: data_dir.clone(),
            snapshot: engine_snapshot.clone(),
            engine_timeout: managed_engine_timeout,
            result_max_file_bytes: managed_result_max_file_bytes,
            failure_analyzer: failure_analyzer.clone(),
        });
        let case_import_service = CaseImportService::new(
            engine_snapshot.clone(),
            upload_batch_service.clone(),
            upload_single_file_limit_bytes,
        );
        let model_patch_service =
            ModelPatchService::new(VersionRepo::new(pool.clone()), artifact_service.clone());
        let dataset_service = DatasetService::new(
            ResultRepo::new(pool.clone()),
            artifact_service.clone(),
            message_repo.clone(),
            events.clone(),
        );
        let chart_service = ChartService::new(
            ResultRepo::new(pool.clone()),
            ChartViewRepo::new(pool.clone()),
            artifact_service.clone(),
        );
        let agent_knowledge = Arc::new(
            AgentKnowledge::load_embedded("v4.4.0").expect("内置 agent knowledge 已在构建时校验"),
        );
        let analysis_service = AnalysisService::new(
            AnalysisRepo::new(pool.clone()),
            ResultRepo::new(pool.clone()),
            artifact_service.clone(),
            message_repo.clone(),
            events.clone(),
            engine.clone(),
            agent_knowledge.clone(),
            data_dir.clone(),
            DataResourceRepo::new(pool.clone()),
        );
        let comparison_service = ComparisonService::new(
            ComparisonRepo::new(pool.clone()),
            RunRepo::new(pool.clone()),
            ResultRepo::new(pool.clone()),
            analysis_service.clone(),
            message_repo.clone(),
            events.clone(),
        );
        let history_service = HistoryService::new(HistoryRepo::new(pool.clone()));
        let study_planning_service = StudyPlanningService::new(
            VersionRepo::new(pool.clone()),
            artifact_service.clone(),
            engine_snapshot.clone(),
        );
        let study_configuration_service = StudyConfigurationService::new(
            StudyConfigurationRepo::new(pool.clone()),
            VersionRepo::new(pool.clone()),
            DataResourceRepo::new(pool.clone()),
            data_resource_service.clone(),
            data_mapping_service.clone(),
            engine_snapshot.clone(),
        );
        let managed_workflow_service = ManagedWorkflowService::new(
            ManagedWorkflowRepo::new(pool.clone()),
            study_configuration_service.clone(),
            events.clone(),
            artifact_service.clone(),
            data_resource_service.clone(),
            &agent_knowledge,
            script_repair_max_attempts,
            script_author.clone(),
            failure_analyzer.clone(),
            engine.clone(),
            engine_snapshot.clone(),
            data_dir.clone(),
            scheduler.clone(),
            session_cancellations.clone(),
            managed_engine_timeout,
            managed_result_max_file_bytes,
            managed_workflow_auto_start,
            ScriptRepo::new(pool.clone()),
        );
        Self {
            pool,
            session_repo,
            message_repo,
            artifact_service,
            version_service,
            model_dataframe_read_service,
            upload_batch_service,
            data_resource_service,
            data_mapping_service,
            script_service,
            run_service,
            case_import_service,
            model_patch_service,
            dataset_service,
            chart_service,
            analysis_service,
            comparison_service,
            history_service,
            study_planning_service,
            study_configuration_service,
            managed_workflow_service,
            engine_snapshot,
            agent_knowledge,
            events,
            engine,
            agent_factory,
            script_author,
            failure_analyzer,
            auth,
            scheduler,
            session_cancellations,
            semaphore: Arc::new(Semaphore::new(4)),
            data_dir,
            upload_single_file_limit_bytes,
            upload_batch_limit_bytes,
        }
    }

    /// 派生一次对话的编排服务（共享引擎/事件/仓储，克隆成本低）。
    pub fn chat(&self, principal: crate::auth::CurrentUser) -> ChatService {
        ChatService::new(self, principal)
    }
}
