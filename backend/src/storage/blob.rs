use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::staging::unique_staging_path;

#[derive(Debug, Clone, Copy)]
pub struct StageLimits {
    pub max_bytes: u64,
}

impl Default for StageLimits {
    fn default() -> Self {
        Self {
            max_bytes: 500 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub struct StorageError {
    code: &'static str,
    message: String,
}

impl StorageError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StorageError {}

#[derive(Debug)]
pub struct StagedArtifact {
    path: Option<PathBuf>,
    pub sha256: String,
    pub size_bytes: u64,
}

impl StagedArtifact {
    pub(crate) fn publish(&mut self, destination: &Path) -> Result<(), StorageError> {
        let Some(staging_path) = self.path.take() else {
            return Err(StorageError::new(
                "artifact_already_published",
                "暂存文件已经发布",
            ));
        };
        if destination.is_file() {
            std::fs::remove_file(&staging_path).map_err(|error| {
                StorageError::new("artifact_staging_cleanup_failed", error.to_string())
            })?;
            return Ok(());
        }
        let parent = destination.parent().ok_or_else(|| {
            StorageError::new("artifact_invalid_path", "artifact 目标路径缺少父目录")
        })?;
        std::fs::create_dir_all(parent)
            .map_err(|error| StorageError::new("artifact_publish_failed", error.to_string()))?;
        std::fs::rename(&staging_path, destination)
            .map_err(|error| StorageError::new("artifact_publish_failed", error.to_string()))?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    }
}

impl Drop for StagedArtifact {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    data_dir: PathBuf,
    staging_dir: PathBuf,
}

impl ArtifactStore {
    pub fn new(data_dir: PathBuf) -> Result<Self, StorageError> {
        let staging_dir = data_dir.join("staging");
        let objects_dir = data_dir.join("objects");
        std::fs::create_dir_all(&staging_dir)
            .and_then(|_| std::fs::create_dir_all(&objects_dir))
            .map_err(|error| StorageError::new("artifact_store_unavailable", error.to_string()))?;
        Ok(Self {
            data_dir,
            staging_dir,
        })
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn stage<R: Read>(
        &self,
        mut reader: R,
        limits: StageLimits,
    ) -> Result<StagedArtifact, StorageError> {
        let path = unique_staging_path(&self.staging_dir);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| StorageError::new("artifact_stage_failed", error.to_string()))?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        let result = (|| -> Result<(), StorageError> {
            loop {
                let read = reader.read(&mut buffer).map_err(|error| {
                    StorageError::new("artifact_read_failed", error.to_string())
                })?;
                if read == 0 {
                    break;
                }
                size = size
                    .checked_add(read as u64)
                    .ok_or_else(|| StorageError::new("artifact_too_large", "artifact 大小溢出"))?;
                if size > limits.max_bytes {
                    return Err(StorageError::new(
                        "artifact_too_large",
                        format!("artifact 超过 {} 字节限制", limits.max_bytes),
                    ));
                }
                hasher.update(&buffer[..read]);
                file.write_all(&buffer[..read]).map_err(|error| {
                    StorageError::new("artifact_stage_failed", error.to_string())
                })?;
            }
            file.sync_all()
                .map_err(|error| StorageError::new("artifact_stage_failed", error.to_string()))
        })();
        if let Err(error) = result {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        let sha256 = format!("{:x}", hasher.finalize());
        Ok(StagedArtifact {
            path: Some(path),
            sha256,
            size_bytes: size,
        })
    }

    pub fn relative_path(sha256: &str) -> String {
        format!("objects/{}/{}", &sha256[..2], sha256)
    }

    pub fn resolve(&self, relative_path: &str) -> Result<PathBuf, StorageError> {
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(StorageError::new(
                "artifact_invalid_path",
                "artifact 路径必须是规范相对路径",
            ));
        }
        Ok(self.data_dir.join(relative))
    }

    pub(crate) fn publish(
        &self,
        staged: &mut StagedArtifact,
        relative_path: &str,
    ) -> Result<(), StorageError> {
        let destination = self.resolve(relative_path)?;
        staged.publish(&destination)
    }

    pub(crate) fn remove(&self, relative_path: &str) -> Result<(), StorageError> {
        let path = self.resolve(relative_path)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StorageError::new(
                "artifact_delete_failed",
                error.to_string(),
            )),
        }
    }
}
