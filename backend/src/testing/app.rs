use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::SimEngine;
use crate::agent::knowledge::AgentKnowledge;
use crate::agent::{AgentFactory, FailureAnalyzer, ScriptAuthor};
use crate::application::{
    AppState, AppStateDependencies, AppStateOptions, RunScheduler, StorageLimits,
};
use crate::auth::AuthService;
use crate::config::RuntimeConfig;
use crate::db::DbPool;
use crate::engine_resources::EngineResourceSnapshot;
use crate::ingestion::ArchiveLimits;

use super::engine_fixture::prepare_engine_fixture;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E2eCatalogMode {
    Minimal,
    Full,
}

pub enum E2eAppProfile {
    Minimal,
    Runtime(Box<RuntimeConfig>),
}

pub struct E2eAppDependencies {
    pub pool: DbPool,
    pub data_dir: PathBuf,
    pub agent_factory: Arc<dyn AgentFactory>,
    pub engine: Arc<dyn SimEngine>,
    pub auth: AuthService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
}

pub struct E2eAppOptions {
    pub scheduler: RunScheduler,
    pub profile: E2eAppProfile,
    pub script_author: Option<Arc<dyn ScriptAuthor>>,
    pub failure_analyzer: Option<Arc<dyn FailureAnalyzer>>,
    pub managed_workflow_auto_start: bool,
    pub catalog_mode: E2eCatalogMode,
}

pub fn build_e2e_app(
    pool: DbPool,
    data_dir: PathBuf,
    agent_factory: Arc<dyn AgentFactory>,
    engine: Arc<dyn SimEngine>,
    auth: AuthService,
    options: E2eAppOptions,
) -> AppState {
    let (engine_snapshot, _) = prepare_engine_fixture(&data_dir, options.catalog_mode);
    let agent_knowledge = Arc::new(
        AgentKnowledge::load_embedded("v4.4.0").expect("load embedded agent knowledge for E2E"),
    );
    crate::tools::validate_tool_catalog(&agent_knowledge.manifest)
        .expect("validate embedded agent tool catalog for E2E");
    assemble_e2e_app(
        E2eAppDependencies {
            pool,
            data_dir,
            agent_factory,
            engine,
            auth,
            engine_snapshot,
            agent_knowledge,
        },
        options,
    )
}

pub fn assemble_e2e_app(dependencies: E2eAppDependencies, options: E2eAppOptions) -> AppState {
    let E2eAppDependencies {
        pool,
        data_dir,
        agent_factory,
        engine,
        auth,
        engine_snapshot,
        agent_knowledge,
    } = dependencies;
    let (
        storage_limits,
        upload_single_file_limit_bytes,
        upload_batch_limit_bytes,
        upload_staging_ttl_hours,
        script_repair_max_attempts,
        agent_model_retry_max,
        managed_engine_timeout,
        managed_result_max_file_bytes,
        archive_limits,
    ) = match options.profile {
        E2eAppProfile::Minimal => (
            StorageLimits {
                artifact_limit: 100,
                byte_limit: 64 * 1024 * 1024,
                warning_ratio: 0.8,
            },
            2 * 1024 * 1024,
            8 * 1024 * 1024,
            24,
            3,
            2,
            std::time::Duration::from_secs(120),
            512 * 1024 * 1024,
            ArchiveLimits {
                max_entries: 100,
                max_expanded_bytes: 16 * 1024 * 1024,
                max_compression_ratio: 20,
                max_file_bytes: 2 * 1024 * 1024,
            },
        ),
        E2eAppProfile::Runtime(config) => (
            StorageLimits {
                artifact_limit: i64::try_from(config.user_artifact_limit)
                    .expect("validated E2E artifact limit fits i64"),
                byte_limit: i64::try_from(config.user_storage_limit_bytes)
                    .expect("validated E2E storage limit fits i64"),
                warning_ratio: config.storage_warning_ratio,
            },
            config.upload_single_file_limit_bytes,
            config.upload_batch_limit_bytes,
            config.upload_staging_ttl_hours,
            config.script_repair_max_attempts,
            config.agent_model_retry_max,
            std::time::Duration::from_millis(config.tensoreval_timeout_ms),
            config.result_max_file_bytes,
            ArchiveLimits {
                max_entries: config.archive_max_entries,
                max_expanded_bytes: config.archive_max_expanded_bytes,
                max_compression_ratio: config.archive_max_compression_ratio,
                max_file_bytes: config.upload_single_file_limit_bytes,
            },
        ),
    };
    AppState::assemble(
        AppStateDependencies {
            pool,
            engine,
            agent_factory,
            agent_knowledge,
            auth,
            scheduler: options.scheduler,
        },
        AppStateOptions {
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
            script_author_override: options.script_author,
            failure_analyzer_override: options.failure_analyzer,
            managed_workflow_auto_start: options.managed_workflow_auto_start,
            archive_limits,
        },
    )
}
