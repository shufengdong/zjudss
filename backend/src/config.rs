//! 运行期配置的集中解析与启动前校验。

use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Instance,
    TrustedProxy,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub database_url: String,
    pub data_dir: PathBuf,
    pub tensoreval_cwd: PathBuf,
    pub auth_mode: AuthMode,
    pub agent_max_concurrent_runs: usize,
    pub agent_max_owner_runs: usize,
    pub agent_queue_capacity: usize,
    pub upload_single_file_limit_bytes: u64,
    pub upload_batch_limit_bytes: u64,
    pub archive_max_entries: usize,
    pub archive_max_expanded_bytes: u64,
    pub archive_max_compression_ratio: u64,
}

#[derive(Debug)]
pub struct ConfigError {
    code: &'static str,
    message: String,
}

impl ConfigError {
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

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ConfigError {}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let base_dir = std::env::current_dir().map_err(|error| {
            ConfigError::new(
                "config_current_dir_unavailable",
                format!("无法读取当前工作目录: {error}"),
            )
        })?;
        Self::from_values(&base_dir, std::env::vars().collect(), None)
    }

    /// 供测试和确定性配置校验使用。test_engine_root 只能由测试夹具显式注入；
    /// 为 None 时采用生产规则，必须从配置中读取 TENSOREVAL_CWD。
    pub fn from_values(
        base_dir: &Path,
        values: BTreeMap<String, String>,
        test_engine_root: Option<PathBuf>,
    ) -> Result<Self, ConfigError> {
        let get = |name: &str| values.get(name).map(String::as_str);

        let database_url = get("DATABASE_URL")
            .unwrap_or("sqlite://backend.db")
            .to_string();
        let data_dir = absolute_path(base_dir, get("DATA_DIR").unwrap_or("data"));
        ensure_writable_dir(&data_dir)?;

        let tensoreval_cwd = match test_engine_root {
            Some(path) => path,
            None => get("TENSOREVAL_CWD").map(PathBuf::from).ok_or_else(|| {
                ConfigError::new(
                    "config_tensoreval_cwd_missing",
                    "非测试启动必须配置 TENSOREVAL_CWD",
                )
            })?,
        };
        if !tensoreval_cwd.is_dir() || std::fs::read_dir(&tensoreval_cwd).is_err() {
            return Err(ConfigError::new(
                "config_tensoreval_cwd_unreadable",
                "TENSOREVAL_CWD 不存在或不是可读目录",
            ));
        }

        let auth_mode = match get("AUTH_MODE").unwrap_or("instance") {
            "instance" => AuthMode::Instance,
            "trusted_proxy" => AuthMode::TrustedProxy,
            value => {
                return Err(ConfigError::new(
                    "config_invalid_value",
                    format!("AUTH_MODE 不支持值 {value}"),
                ));
            }
        };

        Ok(Self {
            database_url,
            data_dir,
            tensoreval_cwd,
            auth_mode,
            agent_max_concurrent_runs: positive_usize(&values, "AGENT_MAX_CONCURRENT_RUNS", 8)?,
            agent_max_owner_runs: positive_usize(&values, "AGENT_MAX_OWNER_RUNS", 2)?,
            agent_queue_capacity: positive_usize(&values, "AGENT_QUEUE_CAPACITY", 32)?,
            upload_single_file_limit_bytes: positive_u64(
                &values,
                "UPLOAD_SINGLE_FILE_LIMIT_BYTES",
                50 * MIB,
            )?,
            upload_batch_limit_bytes: positive_u64(&values, "UPLOAD_BATCH_LIMIT_BYTES", 500 * MIB)?,
            archive_max_entries: positive_usize(&values, "ARCHIVE_MAX_ENTRIES", 1_000)?,
            archive_max_expanded_bytes: positive_u64(
                &values,
                "ARCHIVE_MAX_EXPANDED_BYTES",
                2 * 1024 * MIB,
            )?,
            archive_max_compression_ratio: positive_u64(
                &values,
                "ARCHIVE_MAX_COMPRESSION_RATIO",
                100,
            )?,
        })
    }
}

fn absolute_path(base_dir: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

fn ensure_writable_dir(path: &Path) -> Result<(), ConfigError> {
    std::fs::create_dir_all(path)
        .map_err(|_| ConfigError::new("config_data_dir_unwritable", "DATA_DIR 无法创建或写入"))?;
    let probe = path.join(format!(".zjudss-write-test-{}", std::process::id()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|_| ConfigError::new("config_data_dir_unwritable", "DATA_DIR 无法创建或写入"))?;
    std::fs::remove_file(&probe)
        .map_err(|_| ConfigError::new("config_data_dir_unwritable", "DATA_DIR 写入探针无法清理"))
}

fn positive_usize(
    values: &BTreeMap<String, String>,
    name: &'static str,
    default: usize,
) -> Result<usize, ConfigError> {
    let value = match values.get(name) {
        Some(raw) => raw.parse::<usize>().map_err(|_| invalid_value(name))?,
        None => default,
    };
    if value == 0 {
        return Err(invalid_value(name));
    }
    Ok(value)
}

fn positive_u64(
    values: &BTreeMap<String, String>,
    name: &'static str,
    default: u64,
) -> Result<u64, ConfigError> {
    let value = match values.get(name) {
        Some(raw) => raw.parse::<u64>().map_err(|_| invalid_value(name))?,
        None => default,
    };
    if value == 0 {
        return Err(invalid_value(name));
    }
    Ok(value)
}

fn invalid_value(name: &'static str) -> ConfigError {
    ConfigError::new("config_invalid_value", format!("{name} 必须是大于零的整数"))
}
