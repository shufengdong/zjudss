//! 工具层：把仿真内核能力（模型检查 / 时序仿真）封装成 rig 的有状态工具。
//!
//! 每个工具实例携带绑定到当前 session 的共享服务（引擎适配器 + 事件写入器 + 任务仓储
//! + 限流信号量），**每请求构建一套**（architecture §5 / §10 #2）。工具只通过
//! `adapters::SimEngine` 调用内核，AGENTS.md 硬约束（内核地址/鉴权/错误码收敛在适配器内）
//! 由此落实。

pub mod activate_model_version;
pub mod confirm_dataframe_mapping;
pub mod import_example;
pub mod inspect_example;
pub mod inspect_workspace;
pub mod list_result_datasets;
pub mod load_agent_skill;
pub mod modify_model;
pub mod read_engine_doc_section;
pub mod remove_model_input;
pub mod replace_model_input;
pub mod run_study;
pub mod search_engine_docs;
pub mod search_examples;
mod support;

use std::collections::BTreeSet;
use std::sync::Arc;

use rig::tool::RegisteredTool;
use tokio::sync::{Mutex, Semaphore};

use crate::adapters::SimEngine;
use crate::agent::knowledge::AgentKnowledge;
use crate::application::{
    DatasetService, EventWriter, ExampleImportService, ModelPatchService, SimulationService,
    WorkspaceService,
};
use crate::auth::{AccessScope, Principal};
use crate::db::repositories::TaskRepo;
use crate::engine_resources::EngineResourceSnapshot;

/// 工具共享服务：引擎适配器 + 绑定到当前 session 的写入器 / 任务仓储 / 限流信号量。
#[derive(Clone)]
pub struct ToolServices {
    pub engine: Arc<dyn SimEngine>,
    pub writer: EventWriter,
    pub session_id: i32,
    pub task_repo: TaskRepo,
    pub semaphore: Arc<Semaphore>,
    pub scope: AccessScope,
    pub principal: Principal,
    pub workspaces: WorkspaceService,
    pub example_import: ExampleImportService,
    pub model_patch: ModelPatchService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub loaded_skills: Arc<Mutex<BTreeSet<String>>>,
    pub simulations: SimulationService,
    pub datasets: DatasetService,
}

impl ToolServices {
    pub(crate) async fn require_skill(
        &self,
        skill_id: &str,
    ) -> Result<(), support::DomainToolError> {
        if self.loaded_skills.lock().await.contains(skill_id) {
            Ok(())
        } else {
            Err(support::DomainToolError::new(
                "skill_required",
                format!("请先调用 load_agent_skill 加载 {skill_id}"),
            ))
        }
    }
}

/// 每请求装配一套有状态工具（§5）。
pub fn build_tools(services: Arc<ToolServices>) -> Vec<RegisteredTool> {
    vec![
        RegisteredTool::from_tool(inspect_workspace::InspectWorkspaceTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(load_agent_skill::LoadAgentSkillTool::new(services.clone())),
        RegisteredTool::from_tool(run_study::RunStudyTool::new(services.clone())),
        RegisteredTool::from_tool(list_result_datasets::ListResultDatasetsTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(search_engine_docs::SearchEngineDocsTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(read_engine_doc_section::ReadEngineDocSectionTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(search_examples::SearchExamplesTool::new(services.clone())),
        RegisteredTool::from_tool(inspect_example::InspectExampleTool::new(services.clone())),
        RegisteredTool::from_tool(import_example::ImportExampleTool::new(services.clone())),
        RegisteredTool::from_tool(confirm_dataframe_mapping::ConfirmDataframeMappingTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(replace_model_input::ReplaceModelInputTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(remove_model_input::RemoveModelInputTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(activate_model_version::ActivateModelVersionTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(activate_model_version::RollbackModelVersionTool::new(
            services.clone(),
        )),
        RegisteredTool::from_tool(modify_model::ModifyModelTool::new(services)),
    ]
}
