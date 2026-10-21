//! 仿真内核适配器层：内核与外界的唯一边界（AGENTS.md 硬约束）。
//!
//! 工具层只能调用 [`SimEngine`] 暴露的方法；内核地址、鉴权、子进程、错误码翻译
//! 全部收敛在本模块。当前内核尚未接入，先用 [`StubEngine`] 假实现跑通链路，
//! 后续替换为真实 `tensoreval` 子进程封装（见 `docs/architecture.md` §12 第 4 步）。
//!
//! 调用契约为异步（AGENTS.md）：trait 方法返回 `async`，实现内部负责把同步阻塞的
//! 引擎调用包装成异步（`spawn_blocking` + 子进程），对外不暴露同步阻塞细节。

mod process;
mod stub;

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use async_trait::async_trait;

pub use process::ProcessEngine;
pub use stub::StubEngine;

/// 内核错误分类，供上层按类别处理（前端展示、日志、重试策略）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// 模型 / 脚本解析错误（import 失败、语法错误等）。
    Model,
    /// 求解失败（不收敛、数值发散等）。
    Solver,
    /// 文件 / 进程 IO 错误。
    Io,
    /// 引擎内部错误。
    Internal,
    /// 超时。
    Timeout,
}

impl ErrorCategory {
    /// 稳定的字符串标识，用于落库 / 日志。
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCategory::Model => "model",
            ErrorCategory::Solver => "solver",
            ErrorCategory::Io => "io",
            ErrorCategory::Internal => "internal",
            ErrorCategory::Timeout => "timeout",
        }
    }
}

/// 适配器统一错误：稳定的内核错误码 + 分类 + 可读信息。
///
/// AGENTS.md 约定：禁止把内核原始错误直接抛给上层。这里把内核错误翻译成稳定的
/// `code` 与 `category`，附可读 `message`；`diagnostics` 存结构化引擎诊断（未来落
/// `simulation_tasks.error_json`）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AdapterError {
    pub code: String,
    pub category: ErrorCategory,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<serde_json::Value>,
}

impl AdapterError {
    pub fn new(
        code: impl Into<String>,
        category: ErrorCategory,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            message: message.into(),
            diagnostics: None,
        }
    }

    /// 附加结构化诊断（引擎的 diagnostics/events 等）。
    pub fn with_diagnostics(mut self, diagnostics: serde_json::Value) -> Self {
        self.diagnostics = Some(diagnostics);
        self
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AdapterError {}

/// 模型检查结果（对应引擎 import 步骤，fail-closed）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct InspectOutcome {
    pub ok: bool,
    /// 模型结构摘要 / 检查结论。
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<serde_json::Value>,
}

/// 一次仿真的执行规格（由上层准备，适配器只负责执行）。
#[derive(Debug, Clone)]
pub struct RunSpec {
    /// RSS 脚本路径（`.rss`），由 agent / 后端生成。
    pub script_path: String,
    /// 模型入口（`.dss`）：脚本 `import_dss` 的静态模型。
    pub model_path: String,
    /// 结果输出目录（引擎把每个命名结果写成 `DIR/结果名.csv`）。
    pub result_dir: String,
    /// 请求的结果表名（如 `summary`、`bus_voltage`、`violations`）。
    pub results: Vec<String>,
}

/// 一次仿真的执行结果。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RunOutcome {
    /// 是否收敛（对应 `summary.converged`）。
    pub converged: bool,
    /// 每个命名结果落成的 CSV 路径（结果名 → 路径）。
    pub result_files: BTreeMap<String, String>,
    /// `summary` 关键指标。
    pub metrics: RunMetrics,
}

/// 仿真关键指标（对应引擎 `summary` 表）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RunMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p_loss_w: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub q_loss_var: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_violation_ratio: Option<f64>,
    /// 全网最高电压（p.u.）。真实引擎由 summary/曲线 CSV 聚合，取不到为 `None`。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_voltage: Option<f64>,
    /// 全网最低电压（p.u.）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_voltage: Option<f64>,
    /// 越限时长（秒），数值形式；展示文本（如 "2.5 h"）由工具层格式化。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub violation_duration_s: Option<f64>,
}

/// 单条曲线序列（对应前端 `CurveSeries`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurveSeries {
    pub name: String,
    pub values: Vec<f64>,
}

/// 曲线数据（对应 `frontend/src/types/index.ts` 的 `CurveData`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurveData {
    pub variable: String,
    pub unit: String,
    pub time_axis: Vec<String>,
    pub series: Vec<CurveSeries>,
}

/// 报告文件（二进制下载用）。
#[derive(Debug, Clone)]
pub struct ReportFile {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// 数据/结果根目录（读 `DATA_DIR`，缺省 `"data"`），供引擎与上传路径共用。
pub fn resolve_data_dir() -> PathBuf {
    std::env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data"))
}

/// 仿真内核适配器接口：内核与外界的唯一边界。
///
/// 方法返回 `async`（对外契约，AGENTS.md）；内部由实现负责把同步阻塞的引擎
/// 调用包装成异步（`spawn_blocking` + 子进程）。
#[async_trait]
pub trait SimEngine: Send + Sync {
    /// 模型检查：对应引擎 `import_dss` 解析步骤，fail-closed。
    async fn inspect(&self, model_path: &str) -> Result<InspectOutcome, AdapterError>;

    /// 执行一次仿真，返回收敛状态、结果文件路径与关键指标。
    async fn run(&self, spec: RunSpec) -> Result<RunOutcome, AdapterError>;

    /// 按任务 id 读取曲线数据；无曲线时返回 `Ok(None)`。
    async fn read_curves(&self, task_id: i32) -> Result<Option<CurveData>, AdapterError>;

    /// 按任务 id 读取报告文件；无报告时返回 `Ok(None)`。
    async fn read_report(&self, task_id: i32) -> Result<Option<ReportFile>, AdapterError>;
}

/// 依据环境选择引擎实现：设置了 `TENSOREVAL_BIN` 时用真实子进程引擎，否则回退 stub。
///
/// 引擎二进制当前是 Windows 目标（`tensoreval.exe`），本机无法直接运行；默认 stub
/// 保证开发链路可用，部署时把 `TENSOREVAL_BIN` 指向原生 `tensoreval` 即可切换。
pub fn engine_from_env() -> std::sync::Arc<dyn SimEngine> {
    if std::env::var("TENSOREVAL_BIN").is_ok() {
        std::sync::Arc::new(ProcessEngine::from_env())
    } else {
        std::sync::Arc::new(StubEngine::default())
    }
}
