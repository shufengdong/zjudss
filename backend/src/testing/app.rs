use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::SimEngine;
use crate::agent::{AgentFactory, FailureAnalyzer, ScriptAuthor};
use crate::application::{
    AppState, AppStateDependencies, AppStateOptions, RunScheduler, StorageLimits,
};
use crate::auth::AuthService;
use crate::db::DbPool;
use crate::ingestion::ArchiveLimits;

use super::engine_fixture::prepare_engine_fixture;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E2eCatalogMode {
    Minimal,
    Full,
}

pub struct E2eAppOptions {
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
    AppState::assemble(
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
