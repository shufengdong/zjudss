//! 临时 RSS 任务文件的安全创建与 RAII 清理。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::adapters::{AdapterError, ErrorCategory};

pub(super) struct TemporaryTaskFile {
    path: PathBuf,
    relative_path: PathBuf,
    error_code: &'static str,
    armed: bool,
}

impl TemporaryTaskFile {
    pub(super) fn create(
        root: &Path,
        prefix: &str,
        extension: &str,
        bytes: &[u8],
        error_code: &'static str,
    ) -> Result<Self, AdapterError> {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| {
            AdapterError::new(error_code, ErrorCategory::Internal, error.to_string())
        })?;
        let relative_path = PathBuf::from(format!("{prefix}{}.{}", hex::encode(nonce), extension));
        let path = root.join(&relative_path);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| AdapterError::new(error_code, ErrorCategory::Io, error.to_string()))?;
        if let Err(error) = file.write_all(bytes) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(AdapterError::new(
                error_code,
                ErrorCategory::Io,
                error.to_string(),
            ));
        }
        drop(file);
        Ok(Self {
            path,
            relative_path,
            error_code,
            armed: true,
        })
    }

    pub(super) fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub(super) fn remove(mut self) -> Result<(), AdapterError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(AdapterError::new(
                self.error_code,
                ErrorCategory::Io,
                error.to_string(),
            )),
        }
    }
}

impl Drop for TemporaryTaskFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Component;

    use super::TemporaryTaskFile;

    #[test]
    fn drop_removes_temporary_task_file() {
        let root = tempfile::tempdir().unwrap();
        let path;
        {
            let task_file = TemporaryTaskFile::create(
                root.path(),
                ".zjudss-test-",
                "rss",
                b"result = test();\n",
                "task_file_failed",
            )
            .unwrap();
            path = root.path().join(task_file.relative_path());
            assert_eq!(std::fs::read(&path).unwrap(), b"result = test();\n");
        }
        assert!(!path.exists());
    }

    #[test]
    fn explicit_remove_is_idempotent_for_missing_file() {
        let root = tempfile::tempdir().unwrap();
        let task_file = TemporaryTaskFile::create(
            root.path(),
            ".zjudss-test-",
            "dss",
            b"New Circuit.Test\n",
            "task_file_failed",
        )
        .unwrap();
        let path = root.path().join(task_file.relative_path());
        std::fs::remove_file(&path).unwrap();

        task_file.remove().unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn create_new_files_have_safe_unique_relative_names() {
        let root = tempfile::tempdir().unwrap();
        let user_file = root.path().join(".zjudss-test-user.rss");
        std::fs::write(&user_file, b"user owned").unwrap();
        let first = TemporaryTaskFile::create(
            root.path(),
            ".zjudss-test-",
            "rss",
            b"first",
            "task_file_failed",
        )
        .unwrap();
        let second = TemporaryTaskFile::create(
            root.path(),
            ".zjudss-test-",
            "rss",
            b"second",
            "task_file_failed",
        )
        .unwrap();

        assert_ne!(first.relative_path(), second.relative_path());
        for relative in [first.relative_path(), second.relative_path()] {
            assert_eq!(relative.components().count(), 1);
            assert!(matches!(
                relative.components().next(),
                Some(Component::Normal(_))
            ));
            let name = relative.to_str().unwrap();
            let nonce = name
                .strip_prefix(".zjudss-test-")
                .and_then(|value| value.strip_suffix(".rss"))
                .unwrap();
            assert_eq!(nonce.len(), 32);
            assert!(
                nonce
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            );
        }
        first.remove().unwrap();
        second.remove().unwrap();
        assert_eq!(std::fs::read(&user_file).unwrap(), b"user owned");
    }
}
