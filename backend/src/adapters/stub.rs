//! 假实现：内核未接入前用于跑通链路。
//!
//! `inspect` 返回固定 `model ok`；`run` 返回一个不落盘的假结果。真实 `tensoreval`
//! 子进程封装见 `docs/architecture.md` §12 第 4 步，替换本实现即可，对外签名不变。

use async_trait::async_trait;

use super::{
    AdapterError, CurveData, CurveSeries, InspectOutcome, ReportFile, RunMetrics, RunOutcome,
    RunSpec, SimEngine,
};

/// 假实现：不真正调用引擎。
#[derive(Debug, Clone, Default)]
pub struct StubEngine;

#[async_trait]
impl SimEngine for StubEngine {
    async fn inspect(&self, model_path: &str) -> Result<InspectOutcome, AdapterError> {
        // 与迁移前 main.rs 的硬编码行为保持一致，便于后续替换时对拍。
        let _ = model_path;
        Ok(InspectOutcome {
            ok: true,
            summary: "model ok".to_string(),
            diagnostics: None,
        })
    }

    async fn run(&self, spec: RunSpec) -> Result<RunOutcome, AdapterError> {
        let _ = spec;
        Ok(RunOutcome {
            converged: true,
            result_files: Default::default(),
            // 非零样本：让前端 ResultCard 的电压/越限指标路径真正被走到（真实值待内核接入）。
            metrics: RunMetrics {
                violation_count: Some(3),
                max_voltage: Some(1.082),
                min_voltage: Some(0.941),
                violation_duration_s: Some(9000.0),
                ..Default::default()
            },
        })
    }

    async fn read_curves(&self, task_id: i32) -> Result<Option<CurveData>, AdapterError> {
        let _ = task_id;
        // 样本：96 个 15 分钟步长的时间点 + 两条馈线电压序列，打通前端曲线面板。
        let time_axis: Vec<String> = (0..96)
            .map(|i| {
                let minutes = i * 15;
                format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
            })
            .collect();
        let feeder_a: Vec<f64> = (0..96)
            .map(|i| 1.0 + 0.02 * ((i as f64) * 0.3).sin())
            .collect();
        let feeder_b: Vec<f64> = (0..96)
            .map(|i| 0.98 + 0.015 * ((i as f64) * 0.3 + 1.0).cos())
            .collect();
        Ok(Some(CurveData {
            variable: "电压".to_string(),
            unit: "p.u.".to_string(),
            time_axis,
            series: vec![
                CurveSeries {
                    name: "馈线 A".to_string(),
                    values: feeder_a,
                },
                CurveSeries {
                    name: "馈线 B".to_string(),
                    values: feeder_b,
                },
            ],
        }))
    }

    async fn read_report(&self, task_id: i32) -> Result<Option<ReportFile>, AdapterError> {
        let _ = task_id;
        let md = "# 配电网时序仿真报告\n\n- 收敛：是\n- 最大电压：1.082 p.u.\n\
                  - 最小电压：0.941 p.u.\n- 越限节点数：3\n- 越限时长：2.5 h\n\n\
                  （stub 引擎样本报告）";
        Ok(Some(ReportFile {
            filename: "report.md".to_string(),
            content_type: "text/markdown".to_string(),
            bytes: md.to_string().into_bytes(),
        }))
    }
}
