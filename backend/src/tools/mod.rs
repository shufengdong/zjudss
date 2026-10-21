//! 工具层：把仿真内核能力（模型检查 / 时序仿真）封装成 rig 的有状态工具。
//!
//! 每个工具实例携带绑定到当前 session 的共享服务（引擎适配器 + 事件写入器 + 任务仓储
//! + 限流信号量），**每请求构建一套**（architecture §5 / §10 #2）。工具只通过
//! `adapters::SimEngine` 调用内核，AGENTS.md 硬约束（内核地址/鉴权/错误码收敛在适配器内）
//! 由此落实。

pub mod inspect_model;
pub mod run_simulation;

use std::sync::Arc;

use rig::tool::RegisteredTool;
use tokio::sync::Semaphore;

use crate::adapters::SimEngine;
use crate::application::EventWriter;
use crate::db::repo::TaskRepo;

pub use inspect_model::InspectModelTool;
pub use run_simulation::RunSimulationTool;

/// 工具共享服务：引擎适配器 + 绑定到当前 session 的写入器 / 任务仓储 / 限流信号量。
#[derive(Clone)]
pub struct ToolServices {
    pub engine: Arc<dyn SimEngine>,
    pub writer: EventWriter,
    pub session_id: i32,
    pub task_repo: TaskRepo,
    pub semaphore: Arc<Semaphore>,
}

/// 每请求装配一套有状态工具（§5）。
pub fn build_tools(services: Arc<ToolServices>) -> Vec<RegisteredTool> {
    vec![
        RegisteredTool::from_tool(InspectModelTool::new(services.clone())),
        RegisteredTool::from_tool(RunSimulationTool::new(services)),
    ]
}
