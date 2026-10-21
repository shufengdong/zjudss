//! 真实引擎适配器：把同步的 `tensoreval` CLI 包装成异步子进程调用。
//!
//! 内核是同步单进程 CLI（db-design §0.3）：一次进程 = 一次 case。这里用
//! `tokio::process::Command` 起子进程（引擎的 CPU 密集计算发生在子进程内，不阻塞
//! async worker）；`kill_on_drop(true)` 保证超时/取消时回收子进程。退出码 + stderr
//! 翻译成 [`AdapterError`]（AGENTS.md：禁止把内核原始错误抛给上层）。
//!
//! 调用契约（db-design §0.2 / tensoreval CLI，实测对齐）：
//! - `inspect` → `tensoreval --dss-to-rss --check --study snapshot <model.dss>`
//!   （import 解析 + 静态潮流求解，fail-closed；`--check` 需配 `--study`）。
//! - `run`    → `tensoreval [--df] --result-dir <dir> <input>`，按输入类型选择：
//!   `.rss`（expr_df，加 `--df`）、`.rssbundle`（bundle）、`.dss`（with_dsa 自动转译快照）。
//!
//! 时序（QSTS）仿真由 `.rss` 脚本驱动：脚本经 `import_dss` 导入静态模型，显式定义
//! `solution`（DAILY 等模式 + 步长）与 `load_shape`，再用 `ds_radial_qsts` 求
//! `bus_phase,load_state,summary`（见内核示例 `ieee13_daily_qsts.rss`）。结果 CSV 按
//! RSS 变量名落盘（如 `qsts_summary.csv`/`qsts_voltage.csv`/`qsts_load_state.csv`），
//! 非固定 `summary.csv`。内核 `summary` 为**每时步一行**，本模块按最坏情形聚合。
//! 引擎需以 `with_dsa` + `tensoreval-cli` feature 构建；`TENSOREVAL_BIN` 指定其路径。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::process::Command;

use super::{
    AdapterError, CurveData, CurveSeries, ErrorCategory, InspectOutcome, ReportFile, RunMetrics,
    RunOutcome, RunSpec, SimEngine, resolve_data_dir,
};

/// 引擎调用配置。
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// `tensoreval` 可执行文件路径（或 PATH 上的名字）。
    pub bin: String,
    /// 单次调用超时。
    pub timeout: Duration,
    /// 数据目录（结果 CSV 落在其下子目录，绝对路径）。
    pub data_dir: PathBuf,
    /// 引擎工作目录：`.rss` 脚本里 `import_dss` 的相对路径按此解析（如内核仓库根目录）。
    /// `None` 时继承后端进程 CWD。
    pub cwd: Option<PathBuf>,
}

impl EngineConfig {
    pub fn from_env() -> Self {
        let bin =
            std::env::var("TENSOREVAL_BIN").unwrap_or_else(|_| "tensoreval".to_string());
        let timeout_ms: u64 = std::env::var("TENSOREVAL_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120_000);
        let data_dir = resolve_data_dir();
        let cwd = std::env::var("TENSOREVAL_CWD").ok().map(PathBuf::from);
        Self {
            bin,
            timeout: Duration::from_millis(timeout_ms),
            data_dir,
            cwd,
        }
    }
}

/// 真实子进程引擎。
#[derive(Debug, Clone)]
pub struct ProcessEngine {
    config: EngineConfig,
}

impl ProcessEngine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn from_env() -> Self {
        Self::new(EngineConfig::from_env())
    }

    /// 依据 [`RunSpec`] 决定引擎输入文件与前置参数。
    ///
    /// - 有真实 `.rss` 脚本 → `--df`（expr_df 模式）；
    /// - `.rssbundle` → 无前置参数；
    /// - 否则回落到 `model_path`（`.dss`，with_dsa 自动转译 + 求解）。
    fn resolve_input(spec: &RunSpec) -> (String, Vec<String>) {
        let script = Path::new(&spec.script_path);
        if script.is_file() {
            let ext = script
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext.as_str() {
                "rss" => (spec.script_path.clone(), vec!["--df".to_string()]),
                "rssbundle" => (spec.script_path.clone(), Vec::new()),
                _ => (spec.model_path.clone(), Vec::new()),
            }
        } else {
            (spec.model_path.clone(), Vec::new())
        }
    }

    /// 起子进程执行一次调用；统一处理 spawn 失败与超时。
    async fn run_cmd(&self, args: &[String]) -> Result<std::process::Output, AdapterError> {
        let mut cmd = Command::new(&self.config.bin);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // `.rss` 脚本的 `import_dss` 相对路径按引擎工作目录解析（见 `TENSOREVAL_CWD`）。
        if let Some(cwd) = &self.config.cwd {
            cmd.current_dir(cwd);
        }
        match tokio::time::timeout(self.config.timeout, cmd.output()).await
        {
            Ok(Ok(out)) => Ok(out),
            Ok(Err(e)) => Err(AdapterError::new(
                "spawn_failed",
                ErrorCategory::Io,
                format!("无法启动引擎 {}: {e}", self.config.bin),
            )),
            Err(_) => Err(AdapterError::new(
                "timeout",
                ErrorCategory::Timeout,
                format!("引擎执行超时（>{:?}）", self.config.timeout),
            )),
        }
    }

    /// 非零退出码 → 翻译为 [`AdapterError`]（附退出码与 stderr 结构化诊断）。
    fn exit_error(&self, out: &std::process::Output) -> AdapterError {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let message = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
            .unwrap_or_else(|| {
                format!(
                    "引擎执行失败（退出码 {}）",
                    out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into())
                )
            });
        let (code, category) = classify_error(&stderr);
        AdapterError::new(code, category, message).with_diagnostics(serde_json::json!({
            "exit_code": out.status.code(),
            "stderr": stderr.trim(),
        }))
    }
}

#[async_trait]
impl SimEngine for ProcessEngine {
    async fn inspect(&self, model_path: &str) -> Result<InspectOutcome, AdapterError> {
        let out = self
            .run_cmd(&[
                "--dss-to-rss".to_string(),
                "--check".to_string(),
                "--study".to_string(),
                "snapshot".to_string(),
                model_path.to_string(),
            ])
            .await?;
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let summary = if stdout.trim().is_empty() {
                "model ok".to_string()
            } else {
                stdout.trim().to_string()
            };
            Ok(InspectOutcome {
                ok: true,
                summary,
                diagnostics: None,
            })
        } else {
            Err(self.exit_error(&out))
        }
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutcome, AdapterError> {
        let (input, mut flags) = Self::resolve_input(&spec);
        let result_dir = self.config.data_dir.join(&spec.result_dir);

        let mut args = Vec::new();
        args.append(&mut flags);
        args.push("--result-dir".to_string());
        args.push(result_dir.to_string_lossy().to_string());
        args.push(input);

        let out = self.run_cmd(&args).await?;
        if !out.status.success() {
            return Err(self.exit_error(&out));
        }

        // 引擎退出码 0 表示进程成功，但 `summary.converged` 才是收敛的唯一权威信号
        // （db-design §0.3）；缺失 summary 时按成功处理。
        let (converged, mut metrics) = read_summary(&result_dir);
        // summary.csv 只带损耗/越限计数，不含电压；尽力从曲线 CSV 回填电压极值。
        augment_voltage_metrics(&result_dir, &mut metrics);
        let result_files = collect_result_files(&result_dir, &spec.results);
        Ok(RunOutcome {
            converged,
            result_files,
            metrics,
        })
    }

    async fn read_curves(&self, task_id: i32) -> Result<Option<CurveData>, AdapterError> {
        let dir = self.config.data_dir.join("results").join(task_id.to_string());
        let Some(text) = read_curve_csv(&dir) else {
            return Ok(None);
        };
        Ok(curves_from_csv(&text))
    }

    async fn read_report(&self, task_id: i32) -> Result<Option<ReportFile>, AdapterError> {
        let id = task_id.to_string();

        // 首选：显式落盘的报告文件。
        let report_path = self
            .config
            .data_dir
            .join("reports")
            .join(format!("{id}.md"));
        if let Ok(bytes) = std::fs::read(&report_path) {
            return Ok(Some(ReportFile {
                filename: format!("{id}.md"),
                content_type: "text/markdown".to_string(),
                bytes,
            }));
        }

        // 兜底：结果目录里有 summary CSV（`summary.csv` 或 `*_summary.csv`）才合成 Markdown。
        let dir = self.config.data_dir.join("results").join(&id);
        if find_csv(&dir, &["summary"], "_summary").is_none() {
            return Ok(None);
        }
        let (converged, mut metrics) = read_summary(&dir);
        augment_voltage_metrics(&dir, &mut metrics);
        let md = synthesize_report(&id, converged, &metrics);
        Ok(Some(ReportFile {
            filename: format!("{id}.md"),
            content_type: "text/markdown".to_string(),
            bytes: md.into_bytes(),
        }))
    }
}

/// 从 stderr 推断稳定的错误码与分类。
fn classify_error(stderr: &str) -> (&'static str, ErrorCategory) {
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("converge") || lower.contains("diverge") || lower.contains("solve") {
        ("solver_error", ErrorCategory::Solver)
    } else if lower.contains("parse") || lower.contains("import") || lower.contains("invalid") {
        ("model_error", ErrorCategory::Model)
    } else if lower.contains("unable to read")
        || lower.contains("no such file")
        || lower.contains("unable to create")
        || lower.contains("not found")
    {
        ("io_error", ErrorCategory::Io)
    } else {
        ("engine_error", ErrorCategory::Internal)
    }
}

/// 在结果目录里找 CSV：先按精确 stem（如 `summary`），再按后缀（如 `*_summary`）。
fn find_csv(dir: &Path, exact: &[&str], suffix: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut suffix_match = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("csv") {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if exact.iter().any(|e| e.eq_ignore_ascii_case(&stem)) {
            return Some(path);
        }
        if suffix_match.is_none() && stem.ends_with(&suffix.to_ascii_lowercase()) {
            suffix_match = Some(path);
        }
    }
    suffix_match
}

/// 读取结果目录下的 summary CSV（`summary.csv` 或 `*_summary.csv`，如 `qsts_summary.csv`）；
/// 缺失时按成功处理。
fn read_summary(result_dir: &Path) -> (bool, RunMetrics) {
    let Some(path) = find_csv(result_dir, &["summary"], "_summary") else {
        return (true, RunMetrics::default());
    };
    match std::fs::read_to_string(path) {
        Ok(text) => parse_summary_csv(&text).unwrap_or((true, RunMetrics::default())),
        Err(_) => (true, RunMetrics::default()),
    }
}

/// 按列名（大小写不敏感）取某行单元格，返回的引用生命周期绑定到 `row`。
fn cell_value<'a>(header: &[String], row: &'a [String], name: &str) -> Option<&'a str> {
    let i = header.iter().position(|h| h.eq_ignore_ascii_case(name))?;
    row.get(i).map(|s| s.as_str())
}

/// 解析 summary CSV 文本：首行表头，其后每行一个时步（QSTS）或单行（快照）。
///
/// 多行时按「最坏情形」聚合：`converged` 取 AND（任一时步未收敛即整体未收敛）；
/// `p_loss`/`q_loss`/`violation_count`/`max_violation_ratio` 取跨时步最大值；
/// `violation_duration_s` = 越限时步数 × 步长（步长取相邻 `time` 之差）。
fn parse_summary_csv(text: &str) -> Option<(bool, RunMetrics)> {
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()?
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let rows: Vec<Vec<String>> = lines
        .map(|l| l.split(',').map(|s| s.trim().to_string()).collect())
        .filter(|r: &Vec<String>| r.iter().any(|c| !c.is_empty()))
        .collect();
    if rows.is_empty() {
        return None;
    }

    let parse_f = |row: &[String], names: &[&str]| -> Option<f64> {
        names
            .iter()
            .find_map(|n| cell_value(&header, row, n).and_then(|v| v.parse::<f64>().ok()))
    };

    let mut converged = true;
    let mut p_loss: Option<f64> = None;
    let mut q_loss: Option<f64> = None;
    let mut violation_count: Option<i64> = None;
    let mut max_violation_ratio: Option<f64> = None;
    let mut times: Vec<f64> = Vec::new();
    let mut violating_steps = 0i64;

    for row in &rows {
        let ok = cell_value(&header, row, "converged")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);
        if !ok {
            converged = false;
        }
        if let Some(v) = parse_f(row, &["p_loss", "p_loss_w"]) {
            p_loss = Some(p_loss.map_or(v, |cur| cur.max(v)));
        }
        if let Some(v) = parse_f(row, &["q_loss", "q_loss_var"]) {
            q_loss = Some(q_loss.map_or(v, |cur| cur.max(v)));
        }
        if let Some(vc) = cell_value(&header, row, "violation_count").and_then(|v| v.parse::<i64>().ok()) {
            violation_count = Some(violation_count.map_or(vc, |cur| cur.max(vc)));
            if vc > 0 {
                violating_steps += 1;
            }
        }
        if let Some(v) = parse_f(row, &["max_violation_ratio"]) {
            max_violation_ratio = Some(max_violation_ratio.map_or(v, |cur| cur.max(v)));
        }
        if let Some(t) = parse_f(row, &["time", "time_s"]) {
            times.push(t);
        }
    }

    // 越限时长（秒）：越限时步数 × 步长。单行快照无时间轴时记 None（无时长语义）。
    let step_s = if times.len() >= 2 {
        (times[1] - times[0]).abs()
    } else {
        0.0
    };
    let violation_duration_s = if violating_steps > 0 && step_s > 0.0 {
        Some(violating_steps as f64 * step_s)
    } else {
        None
    };

    Some((
        converged,
        RunMetrics {
            p_loss_w: p_loss,
            q_loss_var: q_loss,
            violation_count,
            max_violation_ratio,
            max_voltage: None,
            min_voltage: None,
            violation_duration_s,
        },
    ))
}

/// 列出结果目录下的 CSV，映射「结果名 → 路径」；`requested` 非空时只保留请求的结果。
fn collect_result_files(dir: &Path, requested: &[String]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("csv") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if requested.is_empty() || requested.iter().any(|r| r == &name) {
            map.insert(name, path.to_string_lossy().to_string());
        }
    }
    map
}

/// 从 load_state / voltage CSV 回填电压极值（p.u.）。
///
/// - `min_voltage`：`*_load_state.csv` 的 `v_min_pu`（内核直接给出的标幺最低电压，精确）。
/// - `max_voltage`：`*_voltage.csv`/`bus_phase.csv` 的 `v_mag`（单位 V）按母线基准归一到 p.u.
///   （见 [`max_voltage_pu`]）。取不到保持 `None`（失败软回退，不报错）。
fn augment_voltage_metrics(result_dir: &Path, metrics: &mut RunMetrics) {
    if metrics.min_voltage.is_none() {
        if let Some(path) = find_csv(result_dir, &["load_state"], "_load_state") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                metrics.min_voltage = csv_column_min(&text, "v_min_pu");
            }
        }
    }
    if metrics.max_voltage.is_none() {
        metrics.max_voltage = max_voltage_pu(result_dir);
    }
}

/// 解析 CSV 文本为（表头, 数据行）；表头按列名规范化（trim），数据行过滤空行。
fn parse_csv_table(text: &str) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()?
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let rows: Vec<Vec<String>> = lines
        .map(|l| l.split(',').map(|s| s.trim().to_string()).collect())
        .filter(|r: &Vec<String>| r.iter().any(|c| !c.is_empty()))
        .collect();
    if header.is_empty() || rows.is_empty() {
        return None;
    }
    Some((header, rows))
}

/// 取 CSV 某一数值列的最小值（列名大小写不敏感）。
fn csv_column_min(text: &str, column: &str) -> Option<f64> {
    let (header, rows) = parse_csv_table(text)?;
    let col = header.iter().position(|h| h.eq_ignore_ascii_case(column))?;
    rows.iter()
        .filter_map(|r| r.get(col))
        .filter_map(|c| c.parse::<f64>().ok())
        .fold(None::<f64>, |acc, v| Some(acc.map_or(v, |a| a.min(v))))
}

/// 找电压曲线 CSV：优先 `bus_phase`/`voltage`/`bus_voltage` 精确名，否则 `*_voltage` 后缀。
fn find_voltage_csv(dir: &Path) -> Option<PathBuf> {
    find_csv(dir, &["bus_phase", "voltage", "bus_voltage"], "_voltage")
}

/// 读母线基准相电压（V）：`*_bus.csv` 的 `voltage_base_v` 列，映射「母线 id → 基准 V」。
fn read_bus_base(dir: &Path) -> Option<BTreeMap<String, f64>> {
    let path = find_csv(dir, &["bus"], "_bus")?;
    let text = std::fs::read_to_string(path).ok()?;
    let (header, rows) = parse_csv_table(&text)?;
    let bus_col = header.iter().position(|h| h.eq_ignore_ascii_case("id") || h.eq_ignore_ascii_case("bus"))?;
    let base_col = header.iter().position(|h| h.eq_ignore_ascii_case("voltage_base_v"))?;
    let mut map = BTreeMap::new();
    for row in &rows {
        let bus = row.get(bus_col)?.clone();
        if let Some(base) = row.get(base_col).and_then(|c| c.parse::<f64>().ok()) {
            if base > 0.0 {
                map.insert(bus, base);
            }
        }
    }
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// 由电压 CSV 计算全网最高电压（p.u.）。
///
/// 电压表的 `v_mag` 单位是 V（相电压）。基准相电压优先按母线取 `voltage_base_v`（精确
/// p.u.）；缺基准时以源母线（各时步最大 `v_mag`，即最高电压等级的平衡节点）为参考——
/// 单电压等级馈线精确，多电压等级为近似。
fn max_voltage_pu(result_dir: &Path) -> Option<f64> {
    let voltage_path = find_voltage_csv(result_dir)?;
    let text = std::fs::read_to_string(voltage_path).ok()?;
    let (header, rows) = parse_csv_table(&text)?;
    let bus_col = header.iter().position(|h| h.eq_ignore_ascii_case("bus"))?;
    let vmag_col = header
        .iter()
        .position(|h| h.eq_ignore_ascii_case("v_mag") || h.eq_ignore_ascii_case("v") || h.eq_ignore_ascii_case("voltage"))?;

    // 每母线取跨时步/相的最大 v_mag（V）。
    let mut max_by_bus: BTreeMap<String, f64> = BTreeMap::new();
    for row in &rows {
        let bus = row.get(bus_col).cloned().unwrap_or_default();
        if let Some(v) = row.get(vmag_col).and_then(|c| c.parse::<f64>().ok()) {
            max_by_bus
                .entry(bus)
                .and_modify(|m| *m = m.max(v))
                .or_insert(v);
        }
    }
    if max_by_bus.is_empty() {
        return None;
    }

    // 优先：按母线基准相电压归一到 p.u.。
    if let Some(base) = read_bus_base(result_dir) {
        let pu = max_by_bus
            .iter()
            .filter_map(|(bus, v)| base.get(bus).map(|b| v / b))
            .fold(None::<f64>, |acc, x| Some(acc.map_or(x, |a| a.max(x))));
        if pu.is_some() {
            return pu;
        }
    }

    // 回退：以源母线（最大 v_mag）为参考。
    let source = max_by_bus.values().cloned().fold(f64::MIN, f64::max);
    if source > 0.0 {
        Some(max_by_bus.values().map(|v| v / source).fold(f64::MIN, f64::max))
    } else {
        None
    }
}

/// 读取结果目录下首个可用的曲线 CSV：优先电压 CSV，否则任意非 summary CSV。
fn read_curve_csv(dir: &Path) -> Option<String> {
    let path = find_voltage_csv(dir).or_else(|| {
        let entries = std::fs::read_dir(dir).ok()?;
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("csv"))
            .filter(|p| {
                let stem = p
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                stem != "summary" && !stem.ends_with("_summary")
            })
            .collect();
        paths.sort();
        paths.into_iter().next()
    })?;
    std::fs::read_to_string(path).ok()
}

/// 从曲线 CSV 文本尽力解析 [`CurveData`]。
///
/// 内核把每个命名结果 DataFrame 写成 `NAME.csv`（polars `CsvWriter` 带表头）；曲线格式
/// 随 RSS 脚本而定，未固化。此处启发式：值列取 `{v_mag,v,p,q,value}` 否则首个数值列；
/// 时间列取 `{time,time_s,t}`；有 time 时以其为 X 轴、按 `{bus,phase,element,name,feeder}`
/// 拼系列（每系列跨时步取一列值），无 time 则每行一个点、合成下标时间轴。失败返回 `None`。
fn curves_from_csv(text: &str) -> Option<CurveData> {
    let (header, rows) = parse_csv_table(text)?;
    let col = |names: &[&str]| -> Option<usize> {
        names
            .iter()
            .find_map(|n| header.iter().position(|h| h.eq_ignore_ascii_case(n)))
    };

    let first_numeric_col = (0..header.len()).find(|&i| {
        rows[0]
            .get(i)
            .and_then(|c| c.parse::<f64>().ok())
            .is_some()
    });
    let value_col = col(&["v_mag", "v", "p", "q", "value"]).or(first_numeric_col)?;
    let value_name = header[value_col].clone();

    let group_cols: Vec<usize> = ["bus", "phase", "element", "name", "feeder"]
        .iter()
        .filter_map(|n| col(std::slice::from_ref(n)))
        .collect();
    let key_of = |row: &[String]| -> String {
        if group_cols.is_empty() {
            "series".to_string()
        } else {
            group_cols
                .iter()
                .filter_map(|&i| row.get(i))
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(".")
        }
    };

    // 有时序列：以 time 为 X 轴，每个系列（bus.phase 等）跨时步取一列值。
    let (time_axis, series) = if let Some(tc) = col(&["time", "time_s", "t"]) {
        let mut times: Vec<String> = rows.iter().filter_map(|r| r.get(tc)).cloned().collect();
        times.sort_by(|a, b| {
            let na = a.parse::<f64>().unwrap_or(0.0);
            let nb = b.parse::<f64>().unwrap_or(0.0);
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
        });
        times.dedup();

        let mut by_key: BTreeMap<String, Vec<Option<f64>>> = BTreeMap::new();
        for row in &rows {
            let t = row.get(tc).cloned().unwrap_or_default();
            let key = key_of(row);
            let v = row.get(value_col).and_then(|c| c.parse::<f64>().ok());
            let entry = by_key
                .entry(key)
                .or_insert_with(|| vec![None; times.len()]);
            if let Some(pos) = times.iter().position(|x| x == &t) {
                entry[pos] = v;
            }
        }
        let series = by_key
            .into_iter()
            .map(|(name, vals)| CurveSeries {
                name,
                values: vals.into_iter().filter_map(|v| v).collect(),
            })
            .collect();
        (times, series)
    } else {
        // 无时间列：每行一个点，合成下标时间轴。
        let mut series: Vec<CurveSeries> = Vec::new();
        let mut order: Vec<String> = Vec::new();
        for row in &rows {
            let key = key_of(row);
            if !order.iter().any(|k| k == &key) {
                order.push(key.clone());
                series.push(CurveSeries {
                    name: key.clone(),
                    values: Vec::new(),
                });
            }
            if let Some(v) = row.get(value_col).and_then(|c| c.parse::<f64>().ok()) {
                if let Some(s) = series.iter_mut().find(|s| s.name == key) {
                    s.values.push(v);
                }
            }
        }
        let time_axis = (0..rows.len()).map(|i| i.to_string()).collect();
        (time_axis, series)
    };

    let (variable, unit) = match value_name.to_ascii_lowercase().as_str() {
        // 内核电压表 `v_mag` 单位是 V（相电压），非标幺。
        "v_mag" | "v" | "voltage" => ("电压".to_string(), "V".to_string()),
        "p" | "p_loss" | "active_power" => ("有功功率".to_string(), String::new()),
        "q" | "q_loss" | "reactive_power" => ("无功功率".to_string(), String::new()),
        other => (other.to_string(), String::new()),
    };

    Some(CurveData {
        variable,
        unit,
        time_axis,
        series,
    })
}

/// 由 summary 关键指标合成一份最小 Markdown 报告（无独立报告文件时的兜底）。
fn synthesize_report(id: &str, converged: bool, metrics: &RunMetrics) -> String {
    let dash = "-".to_string();
    let f64_str = |v: Option<f64>| v.map(|x| x.to_string()).unwrap_or_else(|| dash.clone());
    format!(
        "# 配电网时序仿真报告\n\n\
         - 任务：{id}\n\
         - 收敛：{}\n\
         - 有功损耗：{} W\n\
         - 无功损耗：{} var\n\
         - 越限节点数：{}\n\
         - 最大电压：{} p.u.\n\
         - 最小电压：{} p.u.\n\
         - 越限时长：{}\n",
        if converged { "是" } else { "否" },
        f64_str(metrics.p_loss_w),
        f64_str(metrics.q_loss_var),
        metrics
            .violation_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| dash.clone()),
        f64_str(metrics.max_voltage),
        f64_str(metrics.min_voltage),
        metrics
            .violation_duration_s
            .map(|secs| format!("{:.1} h", secs / 3600.0))
            .unwrap_or_else(|| dash.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_summary_columns() {
        let csv = "converged,p_loss,q_loss,violation_count,max_violation_ratio\n\
                   true,120.5,33.3,2,0.041\n";
        let (converged, metrics) = parse_summary_csv(csv).unwrap();
        assert!(converged);
        assert_eq!(metrics.p_loss_w, Some(120.5));
        assert_eq!(metrics.q_loss_var, Some(33.3));
        assert_eq!(metrics.violation_count, Some(2));
        assert_eq!(metrics.max_violation_ratio, Some(0.041));
    }

    #[test]
    fn parses_summary_aliases_and_bool_forms() {
        let csv = "converged,p_loss_w,q_loss_var,violation_count\n1,10,20,0\n";
        let (converged, metrics) = parse_summary_csv(csv).unwrap();
        assert!(converged);
        assert_eq!(metrics.p_loss_w, Some(10.0));
        assert_eq!(metrics.q_loss_var, Some(20.0));
        assert_eq!(metrics.violation_count, Some(0));
    }

    #[test]
    fn missing_summary_defaults_to_converged() {
        let (converged, metrics) = parse_summary_csv("garbage").unwrap_or((true, RunMetrics::default()));
        assert!(converged);
        assert!(metrics.p_loss_w.is_none());
    }

    #[test]
    fn classifies_common_engine_errors() {
        assert_eq!(classify_error("Failed to parse program: ...").1, ErrorCategory::Model);
        assert_eq!(classify_error("solver did not converge").1, ErrorCategory::Solver);
        assert_eq!(classify_error("unable to read file x").1, ErrorCategory::Io);
        assert_eq!(classify_error("something else").1, ErrorCategory::Internal);
    }

    #[test]
    fn resolves_engine_input() {
        let dir = std::env::temp_dir().join(format!("tensoreval-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let rss = dir.join("case.rss");
        std::fs::write(&rss, "x").unwrap();

        // 真实 .rss 脚本 → 用脚本 + `--df`。
        let spec = RunSpec {
            script_path: rss.to_string_lossy().to_string(),
            model_path: "m.dss".to_string(),
            result_dir: "r".to_string(),
            results: vec![],
        };
        let (input, flags) = ProcessEngine::resolve_input(&spec);
        assert_eq!(input, spec.script_path);
        assert_eq!(flags, vec!["--df".to_string()]);

        // 脚本不存在（如缺省占位 "auto_generated.rss"）→ 回落到 `.dss` 模型。
        let spec2 = RunSpec {
            script_path: "missing.rss".to_string(),
            model_path: "m.dss".to_string(),
            result_dir: "r".to_string(),
            results: vec![],
        };
        let (input2, flags2) = ProcessEngine::resolve_input(&spec2);
        assert_eq!(input2, "m.dss");
        assert!(flags2.is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_multirow_qsts_summary() {
        let csv = "method,converged,p_loss,q_loss,violation_count,max_violation_ratio,time\n\
                   radial,true,100,200,0,,3600\n\
                   radial,true,120,240,2,1.03,7200\n\
                   radial,true,150,300,3,1.15,10800\n";
        let (converged, metrics) = parse_summary_csv(csv).unwrap();
        assert!(converged);
        // 跨时步取最坏（最大）损耗与越限。
        assert_eq!(metrics.p_loss_w, Some(150.0));
        assert_eq!(metrics.q_loss_var, Some(300.0));
        assert_eq!(metrics.violation_count, Some(3));
        assert_eq!(metrics.max_violation_ratio, Some(1.15));
        // 越限步数 2 × 步长 3600s。
        assert_eq!(metrics.violation_duration_s, Some(7200.0));
    }

    #[test]
    fn parses_multirow_summary_not_converged_if_any_step_fails() {
        let csv = "converged,p_loss,q_loss,violation_count,time\n\
                   true,100,200,0,0\n\
                   false,120,240,0,3600\n";
        let (converged, metrics) = parse_summary_csv(csv).unwrap();
        assert!(!converged);
        assert_eq!(metrics.p_loss_w, Some(120.0));
    }

    #[test]
    fn curves_from_csv_builds_series() {
        let csv = "time,bus,phase,v_mag\n\
                   0,bus1,1,1.02\n900,bus1,1,1.01\n0,bus2,1,0.98\n900,bus2,1,0.99\n";
        let curve = curves_from_csv(csv).unwrap();
        assert_eq!(curve.variable, "电压");
        assert_eq!(curve.unit, "V");
        assert_eq!(curve.time_axis, vec!["0", "900"]);
        assert_eq!(curve.series.len(), 2);
        let bus1 = curve.series.iter().find(|s| s.name == "bus1.1").unwrap();
        assert_eq!(bus1.values, vec![1.02, 1.01]);
        let bus2 = curve.series.iter().find(|s| s.name == "bus2.1").unwrap();
        assert_eq!(bus2.values, vec![0.98, 0.99]);
    }

    #[test]
    fn curves_from_csv_missing_time_synthesizes_axis() {
        let csv = "bus,phase,v_mag\nsource,1,1.02\n";
        let curve = curves_from_csv(csv).unwrap();
        assert_eq!(curve.time_axis, vec!["0"]);
        assert_eq!(curve.series.len(), 1);
        assert_eq!(curve.series[0].name, "source.1");
        assert_eq!(curve.series[0].values, vec![1.02]);
    }

    #[test]
    fn augments_voltage_from_load_state_and_voltage() {
        let dir = std::env::temp_dir().join(format!("tensoreval-aug-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // min_voltage 来自 load_state 的 v_min_pu（标幺）。
        std::fs::write(
            dir.join("qsts_load_state.csv"),
            "load,bus,requested_p,served_p,v_min_pu,time\nl1,b1,1,1,0.95,0\nl2,b2,1,1,0.92,0\n",
        )
        .unwrap();
        // max_voltage 来自电压表 v_mag 按源母线（最大 v_mag）归一。
        std::fs::write(
            dir.join("qsts_voltage.csv"),
            "bus,phase,v_mag,time\nsource,1,1.0,0\nb1,1,0.96,0\nb2,1,0.93,0\n",
        )
        .unwrap();
        let mut metrics = RunMetrics::default();
        augment_voltage_metrics(&dir, &mut metrics);
        assert_eq!(metrics.min_voltage, Some(0.92));
        assert_eq!(metrics.max_voltage, Some(1.0));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
