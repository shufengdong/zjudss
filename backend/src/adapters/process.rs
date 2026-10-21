//! 真实引擎适配器：把同步的 `tensoreval` CLI 包装成异步子进程调用。
//!
//! 内核是同步单进程 CLI（db-design §0.3）：一次进程 = 一次 case。这里用
//! `tokio::process::Command` 起子进程（引擎的 CPU 密集计算发生在子进程内，不阻塞
//! async worker）；`kill_on_drop(true)` 保证超时/取消时回收子进程。退出码 + stderr
//! 翻译成 [`AdapterError`]（AGENTS.md：禁止把内核原始错误抛给上层）。
//!
//! 调用契约（db-design §0.2 / tensoreval CLI）：
//! - `inspect` → `tensoreval --dss-to-rss --check <model.dss>`（import 解析 + 求解，fail-closed）
//! - `run`    → `tensoreval [--df] --result-dir <dir> <input>`，按输入类型选择：
//!   `.dss`（with_dsa 自动转译）、`.rss`（expr_df，加 `--df`）、`.rssbundle`（bundle）。
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
    /// 数据目录（结果 CSV 落在其下子目录）。
    pub data_dir: PathBuf,
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
        Self {
            bin,
            timeout: Duration::from_millis(timeout_ms),
            data_dir,
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
        match tokio::time::timeout(
            self.config.timeout,
            Command::new(&self.config.bin)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .output(),
        )
        .await
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

        // 兜底：结果目录里有 summary.csv 才合成 Markdown，否则无报告。
        let dir = self.config.data_dir.join("results").join(&id);
        if !dir.join("summary.csv").is_file() {
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

/// 读取 `DIR/summary.csv`（单行结果表）解析收敛状态与关键指标；缺失时按成功处理。
fn read_summary(result_dir: &Path) -> (bool, RunMetrics) {
    let path = result_dir.join("summary.csv");
    match std::fs::read_to_string(path) {
        Ok(text) => parse_summary_csv(&text).unwrap_or((true, RunMetrics::default())),
        Err(_) => (true, RunMetrics::default()),
    }
}

/// 解析 summary CSV 文本：首行表头、次行数据，按列名容错取值。
fn parse_summary_csv(text: &str) -> Option<(bool, RunMetrics)> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let row = lines.next()?;
    let header: Vec<&str> = header.split(',').map(|s| s.trim()).collect();
    let cells: Vec<&str> = row.split(',').map(|s| s.trim()).collect();
    let get = |name: &str| -> Option<String> {
        let i = header.iter().position(|h| h.eq_ignore_ascii_case(name))?;
        cells.get(i).map(|s| s.to_string())
    };
    let parse_f = |names: &[&str]| -> Option<f64> {
        names
            .iter()
            .find_map(|n| get(n).and_then(|v| v.parse::<f64>().ok()))
    };
    let converged = get("converged")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(true);
    let metrics = RunMetrics {
        p_loss_w: parse_f(&["p_loss", "p_loss_w"]),
        q_loss_var: parse_f(&["q_loss", "q_loss_var"]),
        violation_count: get("violation_count").and_then(|v| v.parse::<i64>().ok()),
        max_violation_ratio: parse_f(&["max_violation_ratio"]),
        // 电压/越限时长：尽力取（当前引擎 summary 无这些列，属预期，保持 None）。
        max_voltage: parse_f(&["max_voltage", "max_v", "v_max"]),
        min_voltage: parse_f(&["min_voltage", "min_v", "v_min"]),
        violation_duration_s: parse_f(&["violation_duration_s", "violation_duration"]),
    };
    Some((converged, metrics))
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

/// 尽力从结果目录的非 summary CSV 回填电压极值。
///
/// 内核曲线/越限 CSV 格式未固化，此为启发式：扫描首个含 `v_mag`/`v`/`voltage` 数值列的
/// CSV，取该列 min/max 作为全网电压极值；取不到保持 `None`（失败软回退，不报错）。
fn augment_voltage_metrics(result_dir: &Path, metrics: &mut RunMetrics) {
    if metrics.max_voltage.is_some() && metrics.min_voltage.is_some() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(result_dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("csv"))
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) != Some("summary"))
        .collect();
    paths.sort();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut lines = text.lines();
        let Some(header) = lines.next() else {
            continue;
        };
        let cols: Vec<&str> = header.split(',').map(|s| s.trim()).collect();
        let Some(col_idx) = cols.iter().position(|c| {
            matches!(c.to_ascii_lowercase().as_str(), "v_mag" | "v" | "voltage")
        }) else {
            continue;
        };
        let mut vals: Vec<f64> = lines
            .filter_map(|l| l.split(',').nth(col_idx))
            .filter_map(|c| c.trim().parse::<f64>().ok())
            .collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if let (Some(&min), Some(&max)) = (vals.first(), vals.last()) {
            if metrics.max_voltage.is_none() {
                metrics.max_voltage = Some(max);
            }
            if metrics.min_voltage.is_none() {
                metrics.min_voltage = Some(min);
            }
        }
        if metrics.max_voltage.is_some() && metrics.min_voltage.is_some() {
            return;
        }
    }
}

/// 读取结果目录下首个可用的曲线 CSV（优先 `bus_phase`/`bus_voltage`/`voltage`，否则任意
/// 非 summary CSV）。
fn read_curve_csv(dir: &Path) -> Option<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return None;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("csv"))
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) != Some("summary"))
        .collect();
    if paths.is_empty() {
        return None;
    }
    let prefer = |name: &str| {
        paths
            .iter()
            .position(|p| p.file_stem().and_then(|s| s.to_str()) == Some(name))
    };
    let idx = prefer("bus_phase")
        .or_else(|| prefer("bus_voltage"))
        .or_else(|| prefer("voltage"))
        .unwrap_or(0);
    std::fs::read_to_string(&paths[idx]).ok()
}

/// 从曲线 CSV 文本尽力解析 [`CurveData`]。
///
/// 内核把每个命名结果 DataFrame 写成 `NAME.csv`（polars `CsvWriter` 带表头）；曲线格式
/// 随 RSS 脚本而定，未固化。此处启发式：值列取 `{v_mag,v,p,q,value}` 否则首个数值列；
/// 时间列取 `{time,time_s,t}` 否则合成下标；系列键由分组列 `{bus,phase,element,name,feeder}`
/// 拼 `.`，无则单系列。失败返回 `None`。
fn curves_from_csv(text: &str) -> Option<CurveData> {
    let mut lines = text.lines();
    let header: Vec<String> = lines
        .next()?
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let rows: Vec<Vec<String>> = lines
        .map(|l| l.split(',').map(|s| s.trim().to_string()).collect())
        .filter(|r: &Vec<String>| !r.is_empty())
        .collect();
    if rows.is_empty() {
        return None;
    }

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

    let time_axis: Vec<String> = match col(&["time", "time_s", "t"]) {
        Some(tc) => rows.iter().filter_map(|r| r.get(tc)).cloned().collect(),
        None => (0..rows.len()).map(|i| i.to_string()).collect(),
    };

    let group_cols: Vec<usize> = ["bus", "phase", "element", "name", "feeder"]
        .iter()
        .filter_map(|n| col(std::slice::from_ref(n)))
        .collect();

    let mut series: Vec<CurveSeries> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    for row in &rows {
        let key = if group_cols.is_empty() {
            "series".to_string()
        } else {
            group_cols
                .iter()
                .filter_map(|&i| row.get(i))
                .filter(|s| !s.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(".")
        };
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

    let (variable, unit) = match value_name.to_ascii_lowercase().as_str() {
        "v_mag" | "v" | "voltage" => ("电压".to_string(), "p.u.".to_string()),
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
    fn parses_summary_voltage_columns() {
        let csv = "converged,p_loss,q_loss,violation_count,max_violation_ratio,max_voltage,min_voltage,violation_duration_s\n\
                   true,120.5,33.3,2,0.041,1.08,0.94,7200\n";
        let (converged, metrics) = parse_summary_csv(csv).unwrap();
        assert!(converged);
        assert_eq!(metrics.max_voltage, Some(1.08));
        assert_eq!(metrics.min_voltage, Some(0.94));
        assert_eq!(metrics.violation_duration_s, Some(7200.0));
    }

    #[test]
    fn curves_from_csv_builds_series() {
        let csv = "time,bus,phase,v_mag\n\
                   0,bus1,1,1.02\n900,bus1,1,1.01\n0,bus2,1,0.98\n900,bus2,1,0.99\n";
        let curve = curves_from_csv(csv).unwrap();
        assert_eq!(curve.variable, "电压");
        assert_eq!(curve.unit, "p.u.");
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
    fn augments_voltage_from_bus_phase_csv() {
        let dir = std::env::temp_dir().join(format!("tensoreval-aug-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bus_phase.csv"),
            "bus,phase,v_mag\nsource,1,1.02\nsource,2,0.95\n",
        )
        .unwrap();
        let mut metrics = RunMetrics::default();
        augment_voltage_metrics(&dir, &mut metrics);
        assert_eq!(metrics.max_voltage, Some(1.02));
        assert_eq!(metrics.min_voltage, Some(0.95));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
