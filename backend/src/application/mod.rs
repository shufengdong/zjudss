//! 编排层：全局状态、事件总线与一次对话的编排。
//!
//! `AppState` 是 axum 的共享状态（连接池、仓储、事件总线、引擎适配器、限流信号量）；
//! `EventBus` 按 session 广播事件供 SSE 订阅；`ChatService` 编排一次 agent 对话。

pub mod analysis;
pub mod artifacts;
pub mod case_import;
pub mod chat;
pub mod comparisons;
mod data_mapping;
pub mod data_resources;
pub(crate) mod dataframe_reader;
pub mod datasets;
mod events;
pub mod history;
pub mod model_dataframes;
pub mod model_patch;
pub mod runs;
pub mod scheduler;
pub mod scripts;
mod study_configurations;
pub mod study_planning;
pub mod uploads;
pub mod versions;
pub mod visualization;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::adapters::{SimEngine, resolve_data_dir};
use crate::agent::knowledge::AgentKnowledge;
use crate::agent::{AgentFactory, DeepSeekAgentFactory, LlmConfig};
use crate::auth::AuthService;
use crate::catalog::SchemaCatalog;
use crate::db::DbPool;
use crate::db::repositories::{
    AnalysisRepo, ArtifactRepo, ComparisonRepo, DataResourceRepo, HistoryRepo, MessageRepo,
    ResultRepo, RunRepo, ScriptRepo, SessionRepo, StorageScopeRepo, StudyConfigurationRepo,
    UploadRepo, VersionRepo,
};
use crate::engine_resources::{EngineKnowledge, EngineResourceSnapshot};
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
    AppEvent, AttachmentFile, DatasetViewRef, EventBus, EventWriter, MESSAGE_TYPES, ModelAmbiguity,
    ModelScope, ModelVersionSnapshot, ModelVersionStatus, OperationKind, OperationRef,
    RecognizedTable, ResultColumn, ResultOutputRef, Stage, StageStatus, ToolStatus, UploadStatus,
    VisualizationKind, ms_to_iso, now_ms,
};
pub use history::{HistoryError, HistoryService};
pub use model_dataframes::{
    DraftReadExpectation, ModelDataFrameQuery, ModelDataFrameQueryResult, ModelDataFrameReadError,
    ModelDataFrameReadService, ModelReadScope,
};
pub use model_patch::{ModelChange, ModelMutationError, ModelPatch, ModelPatchService};
pub use runs::{CreateRunRequest, RunError, RunService, RunSubmission, RunSummary};
pub use scheduler::{AppError, RunScheduler, RunTicket, SchedulerError};
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
    pub analysis_service: AnalysisService,
    pub comparison_service: ComparisonService,
    pub history_service: HistoryService,
    pub study_planning_service: StudyPlanningService,
    pub study_configuration_service: StudyConfigurationService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub events: EventBus,
    pub engine: Arc<dyn SimEngine>,
    pub agent_factory: Arc<dyn AgentFactory>,
    pub auth: AuthService,
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
        let auth = AuthService::new(
            pool.clone(),
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
            false,
        );
        Self::new_with_data_dir_and_agent_factory(
            pool,
            engine,
            agent_factory,
            resolve_data_dir(),
            auth,
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
        auth: AuthService,
    ) -> Self {
        use sha2::{Digest, Sha256};

        use crate::engine_resources::{
            EngineExample, ExampleManifest, ExampleManifestFile, ExampleSource, ExampleSourceFile,
            ExampleStudy,
        };

        let engine_root = data_dir
            .parent()
            .unwrap_or(&data_dir)
            .join("engine-fixture");
        let bundle_path = "E2E/Demo/E2E demo.rssbundle";
        let bundle_relative = format!("dss/rssbundle/{bundle_path}");
        let bundle = engine_root.join(&bundle_relative);
        std::fs::create_dir_all(bundle.join("dss")).expect("create e2e example directory");
        let main_bytes = b"model = import_dss(\"dss/model.dss\");\n";
        let source_bytes = b"new circuit.e2e basekv=10\n";
        std::fs::write(bundle.join("main.rss"), main_bytes).expect("write e2e main");
        std::fs::write(bundle.join("dss/model.dss"), source_bytes).expect("write e2e model");
        let main_hash = hex::encode(Sha256::digest(main_bytes));
        let source_hash = hex::encode(Sha256::digest(source_bytes));
        let manifest = ExampleManifest {
            schema_version: 1,
            artifact: "mems.dss-rss-bundle".into(),
            generator: "zjudss-e2e/1".into(),
            psa_contract_version: "0.1.0".into(),
            entry: "main.rss".into(),
            study: ExampleStudy {
                kind: "execution_plan".into(),
                compatibility: "opendss-native".into(),
                solver: Some("radial".into()),
                results: vec!["voltage".into()],
            },
            source: ExampleSource {
                entry: "dss/model.dss".into(),
                root: "dss".into(),
                tree_sha256: source_hash.clone(),
            },
            files: vec![
                ExampleManifestFile {
                    path: "dss/model.dss".into(),
                    role: "dss-entry".into(),
                    sha256: source_hash.clone(),
                },
                ExampleManifestFile {
                    path: "main.rss".into(),
                    role: "main-rss".into(),
                    sha256: main_hash.clone(),
                },
            ],
            inputs: serde_json::json!([]),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest).expect("serialize e2e manifest");
        std::fs::write(bundle.join("manifest.json"), &manifest_bytes).expect("write e2e manifest");
        let manifest_hash = hex::encode(Sha256::digest(&manifest_bytes));
        let invalid_relative = "dss/rssbundle/E2E/Demo/Broken.rssbundle";
        let invalid_bytes = b"{";
        std::fs::create_dir_all(engine_root.join(invalid_relative))
            .expect("create invalid e2e example directory");
        std::fs::write(
            engine_root.join(invalid_relative).join("manifest.json"),
            invalid_bytes,
        )
        .expect("write invalid e2e manifest");
        let invalid_hash = hex::encode(Sha256::digest(invalid_bytes));
        // macOS 的临时目录常以 /var 暴露、canonicalize 后变为 /private/var；
        // 快照根必须规范化，否则案例导入的越界检查会误判合法 fixture。
        let engine_root = engine_root
            .canonicalize()
            .expect("canonicalize e2e engine root");
        let docs = vec![(
            "docs/e2e-guide.md".to_string(),
            "# E2E guide\nUse deterministic scenarios.".to_string(),
        )];
        let catalog_value = serde_json::json!({
            "schema_version": 2,
            "frames": {
                "bus": {
                    "required": {"id": "str", "kv": "f64"},
                    "optional": {"zone": "str"},
                    "stable_keys": ["id"]
                },
                "load": {
                    "required": {"name": "str", "bus": "str", "p": "f64"},
                    "optional": {"model": "str"},
                    "stable_keys": ["name"]
                },
                "generator": {
                    "required": {"name": "str", "bus": "str", "p": "f64"},
                    "optional": {"dispatch": "str"},
                    "stable_keys": ["name"]
                }
            }
        });
        let catalog = Arc::new(
            SchemaCatalog::from_value("e2e-contract", &catalog_value).expect("e2e catalog valid"),
        );
        let dynamic_catalog = EngineResourceSnapshot::minimal(
            "e2e-snapshot",
            "e2e-contract",
            engine_root.join("dss"),
        );
        let engine_snapshot = Arc::new(EngineResourceSnapshot {
            id: "e2e-snapshot".into(),
            contract_version: "e2e-contract".into(),
            catalog_schema_versions: dynamic_catalog.catalog_schema_versions,
            dataframes_sha256: String::new(),
            functions_sha256: String::new(),
            study_facades_sha256: String::new(),
            source_root: engine_root.join("dss"),
            engine_root: engine_root.clone(),
            catalog_json: catalog_value,
            functions_json: serde_json::json!({"schema_version": 2, "frame_functions": []}),
            study_facades_json: dynamic_catalog.study_facades_json,
            schemas: dynamic_catalog.schemas,
            facades: dynamic_catalog.facades,
            catalog_bytes: Default::default(),
            documents: std::collections::BTreeMap::from([(
                docs[0].0.clone(),
                docs[0].1.as_bytes().to_vec(),
            )]),
            knowledge: EngineKnowledge::from_documents(&docs),
            examples: vec![
                EngineExample {
                    id: "e2e-demo".into(),
                    title: "E2E demo".into(),
                    source_kind: "rssbundle".into(),
                    category_path: vec!["E2E".into(), "Demo".into()],
                    bundle_path: bundle_path.into(),
                    entry_path: Some("main.rss".into()),
                    manifest: Some(manifest),
                    file_count: 3,
                    size_bytes: (manifest_bytes.len() + main_bytes.len() + source_bytes.len())
                        as u64,
                    tree_hash: source_hash.clone(),
                    importable: true,
                    diagnostics: vec![],
                    files: vec![
                        ExampleSourceFile {
                            virtual_path: "manifest.json".into(),
                            source_path: format!("{bundle_relative}/manifest.json"),
                            sha256: manifest_hash,
                            role: "manifest".into(),
                        },
                        ExampleSourceFile {
                            virtual_path: "dss/model.dss".into(),
                            source_path: format!("{bundle_relative}/dss/model.dss"),
                            sha256: source_hash,
                            role: "dss-entry".into(),
                        },
                        ExampleSourceFile {
                            virtual_path: "main.rss".into(),
                            source_path: format!("{bundle_relative}/main.rss"),
                            sha256: main_hash,
                            role: "main-rss".into(),
                        },
                    ],
                },
                EngineExample {
                    id: "e2e-broken".into(),
                    title: "Broken".into(),
                    source_kind: "rssbundle".into(),
                    category_path: vec!["E2E".into(), "Demo".into()],
                    bundle_path: "E2E/Demo/Broken.rssbundle".into(),
                    entry_path: None,
                    manifest: None,
                    file_count: 1,
                    size_bytes: invalid_bytes.len() as u64,
                    tree_hash: invalid_hash.clone(),
                    importable: false,
                    diagnostics: vec!["manifest_invalid".into()],
                    files: vec![ExampleSourceFile {
                        virtual_path: "manifest.json".into(),
                        source_path: format!("{invalid_relative}/manifest.json"),
                        sha256: invalid_hash,
                        role: "manifest".into(),
                    }],
                },
            ],
            source_hashes: Default::default(),
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
            24,
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
        let auth = AuthService::new(
            pool.clone(),
            std::time::Duration::from_secs(30 * 24 * 60 * 60),
            false,
        );
        Self::new_with_data_dir_and_auth(pool, engine, llm, data_dir, auth)
    }

    pub fn new_with_data_dir_and_auth(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
        auth: AuthService,
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
        auth: AuthService,
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
        auth: AuthService,
        scheduler: RunScheduler,
        storage_limits: StorageLimits,
    ) -> Self {
        let catalog = Arc::new(SchemaCatalog::from_value(
            "embedded-test",
            &serde_json::json!({"schema_version":2,"frames":{"table":{"required":{"id":"str"}}}}),
        ).expect("内置最小 catalog 有效"));
        let engine_snapshot = Arc::new(EngineResourceSnapshot::minimal(
            "embedded-empty",
            "embedded-test",
            data_dir.clone(),
        ));
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
            24,
            ArchiveLimits::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_resources(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        llm: LlmConfig,
        data_dir: PathBuf,
        auth: AuthService,
        scheduler: RunScheduler,
        storage_limits: StorageLimits,
        catalog: Arc<SchemaCatalog>,
        engine_snapshot: Arc<EngineResourceSnapshot>,
        upload_single_file_limit_bytes: u64,
        upload_batch_limit_bytes: u64,
        upload_staging_ttl_hours: u64,
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
            upload_staging_ttl_hours,
            archive_limits,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_resources_and_agent_factory(
        pool: DbPool,
        engine: Arc<dyn SimEngine>,
        agent_factory: Arc<dyn AgentFactory>,
        data_dir: PathBuf,
        auth: AuthService,
        scheduler: RunScheduler,
        storage_limits: StorageLimits,
        _catalog: Arc<SchemaCatalog>,
        engine_snapshot: Arc<EngineResourceSnapshot>,
        upload_single_file_limit_bytes: u64,
        upload_batch_limit_bytes: u64,
        upload_staging_ttl_hours: u64,
        archive_limits: ArchiveLimits,
    ) -> Self {
        let session_repo = SessionRepo::new(pool.clone());
        let message_repo = MessageRepo::new(pool.clone());
        let events = EventBus::new();
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
        let upload_batch_service = UploadBatchService::new(
            UploadRepo::new(pool.clone()),
            artifact_service.clone(),
            engine.clone(),
            engine_snapshot.clone(),
            data_dir.clone(),
            upload_staging_ttl_hours,
            archive_limits,
        );
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
            engine: engine.clone(),
            scheduler: scheduler.clone(),
            messages: message_repo.clone(),
            events: events.clone(),
            data_dir: data_dir.clone(),
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
        let agent_knowledge = Arc::new(
            AgentKnowledge::load_embedded("v3.3.3").expect("内置 agent knowledge 已在构建时校验"),
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
            analysis_service,
            comparison_service,
            history_service,
            study_planning_service,
            study_configuration_service,
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
        auth: AuthService,
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
        let engine_snapshot = Arc::new(EngineResourceSnapshot::minimal(
            "embedded-empty",
            "embedded-test",
            data_dir.clone(),
        ));
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
            24,
            ArchiveLimits::default(),
        )
    }

    /// 派生一次对话的编排服务（共享引擎/事件/仓储，克隆成本低）。
    pub fn chat(&self, principal: crate::auth::CurrentUser) -> ChatService {
        ChatService::new(self, principal)
    }
}
