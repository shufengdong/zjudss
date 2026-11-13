//! 应用编排层：装配具名依赖，并把 HTTP/Agent 请求委派给各自拥有状态机的领域门面。
//!
//! 上传、研究配置、托管工作流、普通运行、结果来源、分析和比较保持独立事务与生命周期；
//! `AppState` 只负责共享服务和运行时依赖，不吸收这些领域规则。

pub mod analysis;
pub mod artifacts;
pub mod case_import;
pub mod charting;
pub mod chat;
pub mod comparisons;
pub(crate) mod current_result_context;
mod data_mapping;
pub mod data_resources;
pub(crate) mod dataframe_reader;
pub mod datasets;
pub mod diagnostics;
mod events;
mod execution_failure;
mod execution_input_tree;
mod execution_results;
pub mod history;
pub mod managed_rss;
pub mod managed_workflows;
pub mod model_dataframes;
pub mod model_patch;
mod opaque_token;
mod resource_alias;
mod result_sources;
mod run_results;
pub mod runs;
pub mod scheduler;
pub mod script_contract;
pub mod scripts;
mod struct_overlays;
mod study_configuration_policy;
mod study_configurations;
pub mod study_planning;
pub(crate) mod upload_candidate_policy;
pub mod upload_confirmation;
mod upload_processing;
pub mod uploads;
pub mod versions;
mod workflow_result_contract;
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
use crate::db::DbPool;
use crate::db::repositories::{
    AnalysisRepo, ArtifactRepo, ChartViewRepo, ComparisonRepo, DataResourceRepo, HistoryRepo,
    ManagedWorkflowRepo, MessageRepo, ResultRepo, RunRepo, ScriptRepo, SessionRepo,
    StorageScopeRepo, StudyConfigurationRepo, UploadConfirmationRepo, UploadRepo, VersionRepo,
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
    StructuredReplacementPreview,
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
pub use upload_confirmation::{
    ReplacementAction, UploadConfirmation, UploadConfirmationError, UploadConfirmationService,
};
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
    pub upload_confirmation_service: UploadConfirmationService,
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

/// 生产与测试共同使用的具名外部依赖，避免参数顺序承载隐含语义。
pub struct AppStateDependencies {
    pub pool: DbPool,
    pub engine: Arc<dyn SimEngine>,
    pub agent_factory: Arc<dyn AgentFactory>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub auth: AuthService,
    pub scheduler: RunScheduler,
}

/// 组合根的运行时容量、路径和预检产物。
pub struct AppStateOptions {
    pub data_dir: PathBuf,
    pub storage_limits: StorageLimits,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub upload_single_file_limit_bytes: u64,
    pub upload_batch_limit_bytes: u64,
    pub upload_staging_ttl_hours: u64,
    pub script_repair_max_attempts: usize,
    pub agent_model_retry_max: usize,
    pub managed_result_max_file_bytes: u64,
    pub script_author_override: Option<Arc<dyn ScriptAuthor>>,
    pub failure_analyzer_override: Option<Arc<dyn FailureAnalyzer>>,
    pub managed_workflow_auto_start: bool,
    pub archive_limits: ArchiveLimits,
}

impl AppState {
    #[cfg(feature = "e2e-test")]
    #[doc(hidden)]
    pub async fn stage_struct_overlays_for_test(
        &self,
        user: &crate::auth::CurrentUser,
        plans: &[PreparedStructOverlay],
    ) -> Result<
        (
            Vec<crate::db::repositories::WorkflowOverlayInput>,
            Vec<crate::db::repositories::ArtifactDraft>,
            Vec<crate::storage::StagedArtifact>,
        ),
        (&'static str, String),
    > {
        struct_overlays::StructOverlayService::new(
            DataResourceRepo::new(self.pool.clone()),
            self.data_resource_service.clone(),
            self.artifact_service.clone(),
            self.engine_snapshot.clone(),
        )
        .stage_initial(user, plans)
        .await
        .map(struct_overlays::StagedOverlaySet::into_parts)
        .map_err(|error| (error.code(), error.to_string()))
    }

    /// 用已经通过启动预检的依赖和选项装配所有领域门面。
    pub fn assemble(dependencies: AppStateDependencies, options: AppStateOptions) -> Self {
        let AppStateDependencies {
            pool,
            engine,
            agent_factory,
            agent_knowledge,
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
                confirmation_repo: UploadConfirmationRepo::new(pool.clone()),
                artifacts: artifact_service.clone(),
                engine: engine.clone(),
                snapshot: engine_snapshot.clone(),
                data_dir: data_dir.clone(),
                ttl_hours: upload_staging_ttl_hours,
                archive_limits,
            });
        let data_resource_service = DataResourceService::new(
            DataResourceRepo::new(pool.clone()),
            artifact_service.clone(),
            engine_snapshot.clone(),
        );
        let struct_overlay_service = struct_overlays::StructOverlayService::new(
            DataResourceRepo::new(pool.clone()),
            data_resource_service.clone(),
            artifact_service.clone(),
            engine_snapshot.clone(),
        );
        let upload_confirmation_service = UploadConfirmationService::new(
            DataResourceRepo::new(pool.clone()),
            UploadConfirmationRepo::new(pool.clone()),
            UploadRepo::new(pool.clone()),
            upload_batch_service.clone(),
            data_resource_service.clone(),
            artifact_service.clone(),
            data_dir.clone(),
        );
        let data_mapping_service = DataMappingService::new(
            data_resource_service.clone(),
            upload_confirmation_service.clone(),
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
            struct_overlays: struct_overlay_service.clone(),
            engine: engine.clone(),
            scheduler: scheduler.clone(),
            messages: message_repo.clone(),
            events: events.clone(),
            session_cancellations: session_cancellations.clone(),
            data_dir: data_dir.clone(),
            snapshot: engine_snapshot.clone(),
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
        let managed_workflow_service =
            ManagedWorkflowService::new(managed_workflows::ManagedWorkflowDependencies {
                repo: ManagedWorkflowRepo::new(pool.clone()),
                configurations: study_configuration_service.clone(),
                events: events.clone(),
                artifacts: artifact_service.clone(),
                struct_overlays: struct_overlay_service,
                knowledge: &agent_knowledge,
                correction_budget: script_repair_max_attempts,
                author: script_author.clone(),
                failure_analyzer: failure_analyzer.clone(),
                engine: engine.clone(),
                snapshot: engine_snapshot.clone(),
                data_dir: data_dir.clone(),
                scheduler: scheduler.clone(),
                session_cancellations: session_cancellations.clone(),
                result_max_file_bytes: managed_result_max_file_bytes,
                auto_start: managed_workflow_auto_start,
                scripts: ScriptRepo::new(pool.clone()),
            });
        Self {
            pool,
            session_repo,
            message_repo,
            artifact_service,
            version_service,
            model_dataframe_read_service,
            upload_batch_service,
            upload_confirmation_service,
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
