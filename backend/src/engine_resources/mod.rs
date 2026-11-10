//! 当前 `SPARROWZZ_DIR` 资源树的只读加载、验证、快照和不可变发布边界。
//!
//! 服务启动时从文件字节计算快照身份并发布副本；源树不会被改写，资源更新需重启服务后才生效。

mod examples;
mod facades;
mod knowledge;
mod loader;
mod projections;
mod publisher;
mod snapshot;
mod tree;

use std::path::{Path, PathBuf};

pub use examples::{
    EngineExample, ExampleManifest, ExampleManifestFile, ExampleProjection,
    ExampleProjectionResult, ExampleSource, ExampleSourceFile, ExampleStudy,
};
pub use facades::{
    DisabledFacadeDiagnostic, FacadeContract, FacadeInputContract, FacadeInputKind, FacadeRegistry,
    FacadeResultContract, FacadeSearchResult, ResultCondition, ResultSelectionRule,
};
pub use knowledge::{DocSearchHit, DocSection, EngineKnowledge};
pub use projections::{
    CaseProjectionProfile, CaseProjectionRegistry, ProjectorOutputContract,
    ProjectorOutputDisposition,
};
pub use snapshot::{CatalogSchemaVersions, EngineResourceSnapshot, ResourceLimits};

pub const SUPPORTED_FUNCTIONS_SCHEMA_VERSION: u32 = 2;
pub const SUPPORTED_STUDY_FACADES_SCHEMA_VERSION: u32 = 1;
pub const SUPPORTED_CASE_PROJECTIONS_SCHEMA_VERSION: u32 = 2;
pub const SUPPORTED_DATAFRAMES_SCHEMA_VERSION: u32 = 4;
pub const SUPPORTED_DTYPE_GRAMMAR_VERSION: u32 = 1;

#[derive(Debug)]
pub struct EngineResourceError {
    code: &'static str,
    message: String,
}

impl EngineResourceError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn io(error: std::io::Error) -> Self {
        Self::new("engine_resource_io", error.to_string())
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for EngineResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EngineResourceError {}

/// 负责从当前资源树构造并发布一次启动期不可变快照。
pub struct EngineResourceRegistry;

impl EngineResourceRegistry {
    pub fn load(sparrowzz_dir: &Path) -> Result<EngineResourceSnapshot, EngineResourceError> {
        Self::load_with_limits(sparrowzz_dir, ResourceLimits::default())
    }

    pub fn load_with_limits(
        sparrowzz_dir: &Path,
        limits: ResourceLimits,
    ) -> Result<EngineResourceSnapshot, EngineResourceError> {
        loader::load_snapshot(sparrowzz_dir, limits)
    }

    pub fn publish(
        snapshot: EngineResourceSnapshot,
        data_dir: &Path,
    ) -> Result<(EngineResourceSnapshot, PathBuf), EngineResourceError> {
        publisher::publish_snapshot(snapshot, data_dir)
    }
}
