//! `run_simulation` 工具：把一次时序仿真编排成有状态任务（architecture §6）。
//!
//! 职责：限流（信号量）→ 建 `simulation_tasks` 行（pending）→ 调适配器 `SimEngine::run()`
//! → 推进任务状态机（pending → running → succeeded/failed）并落库 → 发 progress/result
//! 事件。内核子进程、错误码翻译都收敛在适配器内（AGENTS.md 硬约束）。

use std::sync::Arc;

use rig::tool::{Tool, ToolContext};

use crate::adapters::{AdapterError, ErrorCategory, RunSpec};
use crate::application::{Metrics, Stage, StageStatus, now_ms};
use crate::db::models::NewSimulationTask;
use crate::tools::ToolServices;

/// 时序仿真工具的参数。
#[derive(serde::Deserialize)]
pub struct RunSimulationArgs {
    /// 模型文件路径（.dss）。
    pub model_path: String,
    /// 仿真脚本路径（.rss / .rssbundle）；缺省时直接跑 `.dss`（引擎自动转译）。
    #[serde(default)]
    pub script_path: Option<String>,
    /// 仿真时长（秒），当前 stub 引擎暂不使用。
    #[serde(default)]
    pub duration_s: Option<f64>,
    /// 步长（秒），当前 stub 引擎暂不使用。
    #[serde(default)]
    pub step_s: Option<f64>,
}

pub struct RunSimulationTool {
    services: Arc<ToolServices>,
}

impl RunSimulationTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

fn stage(key: &str, label: &str, status: StageStatus) -> Stage {
    Stage {
        key: key.to_string(),
        label: label.to_string(),
        status,
    }
}

/// 前端 progress 的四个阶段（对齐 architecture §6 的 inspect/sim/analyze/report）。
///
/// 注意：这与 `simulation_tasks.stage`（`model_check|script_gen|running|collecting`）是**两套
/// 状态**，前者是出网 UI 阶段，后者是任务内部阶段机（决策 D7），各自推进、互不合并。
fn progress_stages(sim: StageStatus, analyze: StageStatus, report: StageStatus) -> Vec<Stage> {
    vec![
        stage("inspect", "模型检查", StageStatus::Done),
        stage("sim", "时序仿真", sim),
        stage("analyze", "结果分析", analyze),
        stage("report", "报告生成", report),
    ]
}

/// 把仓储错误翻译为适配器错误（工具 `Error` 类型是 `AdapterError`）。
fn db_err(e: crate::db::repo::RepoError) -> AdapterError {
    AdapterError::new(
        "db_error",
        ErrorCategory::Internal,
        format!("任务落库失败: {e}"),
    )
}

/// 序列化适配器错误为结构化诊断（落 `simulation_tasks.error_json`）。
fn error_json(err: &AdapterError) -> Option<String> {
    serde_json::to_string(err).ok()
}

/// 越限时长（秒）→ 展示用字符串：`0.0` 记「0 h」，否则按小时一位小数。
fn format_duration(secs: f64) -> String {
    if secs <= 0.0 {
        "0 h".to_string()
    } else {
        format!("{:.1} h", secs / 3600.0)
    }
}

impl Tool for RunSimulationTool {
    const NAME: &'static str = "run_simulation";
    type Args = RunSimulationArgs;
    type Output = String;
    type Error = AdapterError;

    fn description(&self) -> String {
        "对配电网模型执行时序（QSTS）仿真，返回 task_id 与关键指标（损耗/电压越限等）。\
         时序仿真由 .rss 脚本驱动：脚本需 import_dss 导入模型，并用 ds_radial_qsts 请求 \
         bus_phase,load_state,summary 三个结果表，适配器据此聚合指标。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "model_path": {
                    "type": "string",
                    "description": "配电网模型文件路径（.dss）"
                },
                "script_path": {
                    "type": "string",
                    "description": "时序仿真脚本路径（.rss）；须经 import_dss 导入模型，并以 ds_radial_qsts 请求 bus_phase,load_state,summary 结果表才能产出电压/越限指标"
                },
                "duration_s": {
                    "type": "number",
                    "description": "仿真时长（秒）；当前由 .rss 脚本的 solution/load_shape 决定，此参数仅作记录"
                },
                "step_s": {
                    "type": "number",
                    "description": "仿真步长（秒）；当前由 .rss 脚本的 solution 决定，此参数仅作记录"
                }
            },
            "required": ["model_path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let services = &self.services;

        // 1. 限流：引擎为 CPU 密集的同步批处理（db-design §0.5），控制并发子进程数。
        let _permit = services
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                AdapterError::new(
                    "overloaded",
                    ErrorCategory::Internal,
                    "并发仿真任务已满，请稍后重试",
                )
            })?;

        let now = now_ms();

        // 2. 建任务行（pending），task_id 即 `simulation_tasks.id`。
        let params = serde_json::json!({
            "model_path": args.model_path,
            "script_path": args.script_path,
            "duration_s": args.duration_s,
            "step_s": args.step_s,
        });
        let task = services
            .task_repo
            .create(NewSimulationTask {
                session_id: services.session_id,
                status: "pending".to_string(),
                stage: Some("model_check".to_string()),
                study_type: None,
                params_json: params.to_string(),
                exit_code: None,
                error_json: None,
                created_at_ms: now,
                started_at_ms: None,
                finished_at_ms: None,
            })
            .await
            .map_err(db_err)?;
        let task_id = task.id;
        let task_id_str = task.id.to_string();

        // 3. 引擎执行前：progress 停在「时序仿真 running」。
        let _ = services
            .writer
            .progress(
                task_id,
                progress_stages(
                    StageStatus::Running,
                    StageStatus::Pending,
                    StageStatus::Pending,
                ),
            )
            .await;

        // 4. 状态机 → running，调适配器执行。
        let _ = services
            .task_repo
            .mark_running(task.id, "running", now_ms())
            .await
            .map_err(db_err)?;

        let script_path = args
            .script_path
            .unwrap_or_else(|| "auto_generated.rss".to_string());
        let result_dir = format!("results/{task_id_str}");
        let outcome = services
            .engine
            .run(RunSpec {
                script_path,
                model_path: args.model_path,
                result_dir,
                // 结果表名随 RSS 变量名而定（如 qsts_summary/qsts_voltage/qsts_load_state），
                // 留空表示收集全部 CSV，解析时按后缀/列名容错识别。
                results: vec![],
            })
            .await;

        match outcome {
            Ok(outcome) if outcome.converged => {
                // 5a. 成功：succeeded + progress 全 done + result 事件。
                let _ = services
                    .task_repo
                    .mark_done(task.id, "succeeded", "collecting", Some(0), None, now_ms())
                    .await;
                let _ = services
                    .writer
                    .progress(
                        task_id,
                        progress_stages(
                            StageStatus::Done,
                            StageStatus::Done,
                            StageStatus::Done,
                        ),
                    )
                    .await;

                // 电压指标来自适配器聚合（summary/bus_phase 尽力解析），缺失时回退 0。
                let m = &outcome.metrics;
                let metrics = Metrics {
                    max_voltage: m.max_voltage.unwrap_or(0.0),
                    min_voltage: m.min_voltage.unwrap_or(0.0),
                    violation_nodes: m.violation_count.unwrap_or(0),
                    violation_duration: m
                        .violation_duration_s
                        .map(format_duration)
                        .unwrap_or_else(|| "0 h".to_string()),
                };
                let _ = services
                    .writer
                    .result(task_id, "配电网时序仿真结果", metrics)
                    .await;

                Ok(format!("task_id: {task_id_str}"))
            }
            Ok(_outcome) => {
                // 5b. 退出码 0 但 summary.converged=false（求解未收敛）。
                let err =
                    AdapterError::new("not_converged", ErrorCategory::Solver, "时序仿真未收敛");
                let _ = services
                    .task_repo
                    .mark_done(task.id, "failed", "running", None, error_json(&err), now_ms())
                    .await;
                let _ = services
                    .writer
                    .progress(
                        task_id,
                        progress_stages(
                            StageStatus::Failed,
                            StageStatus::Pending,
                            StageStatus::Pending,
                        ),
                    )
                    .await;
                Err(err)
            }
            Err(err) => {
                // 5c. 引擎失败：failed + progress 标 failed，原样返回适配器错误。
                let _ = services
                    .task_repo
                    .mark_done(task.id, "failed", "running", None, error_json(&err), now_ms())
                    .await;
                let _ = services
                    .writer
                    .progress(
                        task_id,
                        progress_stages(
                            StageStatus::Failed,
                            StageStatus::Pending,
                            StageStatus::Pending,
                        ),
                    )
                    .await;
                Err(err)
            }
        }
    }
}
