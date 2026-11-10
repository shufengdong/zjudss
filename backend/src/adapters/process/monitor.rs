//! 引擎运行期间的结果目录安全与容量监控。

use std::path::Path;
use std::time::Duration;

use crate::adapters::AdapterError;

use super::OutputLimits;
use super::result_scan::{ScanPhase, scan_result_dir};

pub(crate) async fn monitor_result_dir(
    result_dir: &Path,
    limits: OutputLimits,
) -> Result<(), AdapterError> {
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        scan_result_dir(result_dir, limits, ScanPhase::Monitoring)?;
    }
}

pub(crate) fn scan_result_limits(
    result_dir: &Path,
    limits: OutputLimits,
    monitoring: bool,
) -> Result<(), AdapterError> {
    scan_result_dir(
        result_dir,
        limits,
        if monitoring {
            ScanPhase::Monitoring
        } else {
            ScanPhase::Final
        },
    )?;
    Ok(())
}
