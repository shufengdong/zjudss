//! artifact 物理存储边界：有界 staging、内容寻址发布与 Parquet 事实检查。
//!
//! 引用计数、配额和领域血缘属于 repository；本层不决定资源生命周期。

mod blob;
mod parquet;
mod staging;

pub use blob::{ArtifactStore, StageLimits, StagedArtifact, StagingWriter, StorageError};
pub use parquet::{ParquetColumn, ParquetError, ParquetInspector, ParquetMetadata};
