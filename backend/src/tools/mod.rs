//! 工具层：把仿真内核能力（模型检查 / 时序仿真）封装成 rig 的有状态工具。
//!
//! 每个工具实例携带绑定到当前 session 的共享服务（引擎适配器、事件写入器、任务仓储
//! 和限流信号量），**每请求构建一套**（architecture §5 / §10 #2）。
//!
//! 工具只通过 `adapters::SimEngine` 调用内核。内核地址、鉴权和错误码翻译均由适配器
//! 收敛，以落实 AGENTS.md 的边界约束。

pub mod compare_simulations;
pub mod create_dataset_view;
pub mod create_script;
pub mod get_storage_usage;
pub mod inspect_example;
pub mod inspect_workspace;
pub mod list_result_datasets;
pub mod list_simulation_history;
pub mod load_agent_skill;
pub mod modify_model;
pub mod read_chat_context;
pub mod read_engine_doc_section;
pub mod read_model_dataframe;
pub mod run_result_analysis;
pub mod search_chat_history;
pub mod search_engine_docs;
pub mod search_examples;
mod support;

use std::collections::BTreeSet;
use std::sync::Arc;

use rig::tool::{RegisteredTool, Tool};
use tokio::sync::{Mutex, Semaphore};

use crate::adapters::SimEngine;
use crate::agent::knowledge::AgentKnowledge;
use crate::application::{
    AnalysisService, ArtifactService, ComparisonService, DatasetService, EventWriter,
    HistoryService, ModelDataFrameReadService, ModelPatchService, RunService, ScriptService,
    VersionService,
};
use crate::auth::{CurrentUser, UserId};
use crate::engine_resources::EngineResourceSnapshot;

/// 工具共享服务：引擎适配器 + 绑定到当前 session 的写入器 / 任务仓储 / 限流信号量。
#[derive(Clone)]
pub struct ToolServices {
    pub engine: Arc<dyn SimEngine>,
    pub writer: EventWriter,
    pub session_id: i32,
    pub request_message_id: i32,
    pub semaphore: Arc<Semaphore>,
    pub scope: UserId,
    pub principal: CurrentUser,
    pub model_patch: ModelPatchService,
    pub model_dataframes: ModelDataFrameReadService,
    pub scripts: ScriptService,
    pub versions: VersionService,
    pub runs: RunService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub loaded_skills: Arc<Mutex<BTreeSet<String>>>,
    pub datasets: DatasetService,
    pub analysis: AnalysisService,
    pub comparisons: ComparisonService,
    pub history: HistoryService,
    pub artifact_service: ArtifactService,
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

macro_rules! define_tool_registry {
    ($( $module:ident :: $tool:ident ),+ $(,)?) => {
        pub fn registered_tool_names() -> &'static [&'static str] {
            &[$(<$module::$tool as Tool>::NAME),+]
        }

        /// 每请求装配一套有状态工具（§5）。
        pub fn build_tools(services: Arc<ToolServices>) -> Vec<RegisteredTool> {
            vec![$(
                RegisteredTool::from_tool($module::$tool::new(services.clone()))
            ),+]
        }
    };
}

define_tool_registry!(
    inspect_workspace::InspectWorkspaceTool,
    read_model_dataframe::ReadModelDataFrameTool,
    search_examples::SearchExamplesTool,
    inspect_example::InspectExampleTool,
    modify_model::ModifyModelTool,
    create_script::CreateScriptTool,
    list_result_datasets::ListResultDatasetsTool,
    run_result_analysis::RunResultAnalysisTool,
    create_dataset_view::CreateDatasetViewTool,
    compare_simulations::CompareSimulationsTool,
    list_simulation_history::ListSimulationHistoryTool,
    search_chat_history::SearchChatHistoryTool,
    read_chat_context::ReadChatContextTool,
    get_storage_usage::GetStorageUsageTool,
    load_agent_skill::LoadAgentSkillTool,
    search_engine_docs::SearchEngineDocsTool,
    read_engine_doc_section::ReadEngineDocSectionTool,
);
