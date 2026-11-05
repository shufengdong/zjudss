//! 工具层：把仿真内核能力（模型检查 / 时序仿真）封装成 rig 的有状态工具。
//!
//! 每个工具实例携带绑定到当前 session 的共享服务（引擎适配器、事件写入器、任务仓储
//! 和限流信号量），**每请求构建一套**（architecture §5 / §10 #2）。
//!
//! 工具只通过 `adapters::SimEngine` 调用内核。内核地址、鉴权和错误码翻译均由适配器
//! 收敛，以落实 AGENTS.md 的边界约束。

pub mod compare_simulations;
pub mod get_storage_usage;
pub mod inspect_example;
pub mod inspect_workspace;
pub mod list_data_resources;
pub mod list_result_datasets;
pub mod list_simulation_history;
pub mod load_agent_skill;
pub mod modify_model;
pub mod prepare_study;
pub mod promote_result_dataset;
pub mod publish_model_version;
pub mod read_chat_context;
pub mod read_data_resource;
pub mod read_engine_doc_section;
pub mod read_model_dataframe;
pub mod run_result_analysis;
pub mod search_chat_history;
pub mod search_engine_docs;
pub mod search_examples;
pub mod search_study_facades;
mod support;
pub mod update_data_resource_alias;
pub mod update_study_results;

use std::collections::BTreeSet;
use std::sync::Arc;

use rig::tool::{RegisteredTool, Tool};
use tokio::sync::{Mutex, Semaphore};

use crate::adapters::SimEngine;
use crate::agent::knowledge::{AgentKnowledge, AgentManifest};
use crate::application::{
    AnalysisService, ArtifactService, ComparisonService, DataResourceService, DatasetService,
    EventWriter, HistoryService, ModelDataFrameReadService, ModelPatchService, RunService,
    StudyConfigurationService, VersionService,
};
use crate::auth::{CurrentUser, UserId};
use crate::db::models::DraftRef;
use crate::engine_resources::EngineResourceSnapshot;

/// 工具共享服务：引擎适配器 + 绑定到当前 session 的写入器 / 任务仓储 / 限流信号量。
#[derive(Clone)]
pub struct ToolServices {
    pub engine: Arc<dyn SimEngine>,
    pub writer: EventWriter,
    pub session_id: i32,
    pub request_message_id: i32,
    pub publication_authorized: bool,
    pub semaphore: Arc<Semaphore>,
    pub scope: UserId,
    pub principal: CurrentUser,
    pub model_patch: ModelPatchService,
    pub model_dataframes: ModelDataFrameReadService,
    pub versions: VersionService,
    pub runs: RunService,
    pub engine_snapshot: Arc<EngineResourceSnapshot>,
    pub agent_knowledge: Arc<AgentKnowledge>,
    pub loaded_skills: Arc<Mutex<BTreeSet<String>>>,
    pub inspected_workspace_draft: Arc<Mutex<Option<DraftRef>>>,
    pub study_configurations: StudyConfigurationService,
    pub datasets: DatasetService,
    pub analysis: AnalysisService,
    pub comparisons: ComparisonService,
    pub history: HistoryService,
    pub artifact_service: ArtifactService,
    pub data_resources: DataResourceService,
}

/// 只授权当前用户消息里明确、肯定的模型发布请求。
///
/// 这是工具层的纵深防御；Agent Prompt 仍负责引导模型理解用户意图。
pub fn publication_intent_authorized(prompt: &str) -> bool {
    let normalized = prompt.trim().to_lowercase();
    let rejected_markers = [
        "不要",
        "不准",
        "不得",
        "不允许",
        "不应",
        "不能",
        "不可",
        "不发布",
        "别",
        "禁止",
        "无需",
        "无须",
        "永不",
        "永远不",
        "是否",
        "能否",
        "可不可以",
        "可以吗",
        "如何",
        "怎么",
        "告诉我",
        "说明",
        "解释",
        "在哪里",
        "哪里",
        "哪些",
        "什么",
        "步骤",
        "哪些步骤",
        "需要什么",
        "需要哪些",
        "取消",
        "撤销",
        "放弃",
        "算了",
        "并非",
        "不是",
        "?",
        "？",
        "do not",
        "don't",
        "never",
        "without",
        "can you",
        "could you",
        "would you",
        "how to",
        "how do",
        "cancel",
        "abort",
        "stop",
    ];
    let has_not_word = normalized
        .split(|character: char| !character.is_ascii_alphabetic())
        .any(|word| word == "not");
    if has_not_word
        || rejected_markers
            .iter()
            .any(|marker| normalized.contains(marker))
    {
        return false;
    }

    let chinese_commands = [
        "请发布当前模型草稿",
        "发布当前模型草稿",
        "请帮我发布当前模型草稿",
        "帮我发布当前模型草稿",
        "立即发布当前模型草稿",
        "现在发布当前模型草稿",
        "先发布当前模型草稿",
        "请先发布当前模型草稿",
        "请发布当前模型版本",
        "发布当前模型版本",
        "请帮我发布当前模型版本",
        "帮我发布当前模型版本",
        "立即发布当前模型版本",
        "现在发布当前模型版本",
        "先发布当前模型版本",
        "请先发布当前模型版本",
        "请发布模型版本",
        "发布模型版本",
        "发布草稿",
        "请发布草稿",
        "请生成当前模型版本",
        "生成当前模型版本",
        "请生成模型版本",
        "生成模型版本",
    ];
    let terminal_suffix = |suffix: &str| ["", "。", ".", "！", "!"].contains(&suffix);
    let description_prefixes = ["，版本描述为", ",版本描述为", "，描述为", ",描述为"];
    let then_prefixes = ["，然后", ",然后", "然后"];
    for command in chinese_commands {
        if let Some(suffix) = normalized.strip_prefix(command) {
            if terminal_suffix(suffix)
                || description_prefixes
                    .iter()
                    .any(|prefix| suffix.starts_with(prefix))
            {
                return true;
            }
            for activation_prefix in ["并自动激活", "，并自动激活", ",并自动激活"] {
                if let Some(tail) = suffix.strip_prefix(activation_prefix) {
                    return terminal_suffix(tail)
                        || description_prefixes
                            .iter()
                            .any(|prefix| tail.starts_with(prefix));
                }
            }
            for then_prefix in then_prefixes {
                if let Some(action) = suffix.strip_prefix(then_prefix) {
                    return ["使用", "准备", "执行"]
                        .iter()
                        .any(|verb| action.starts_with(verb));
                }
            }
            return false;
        }
    }

    let english_commands = [
        "please publish the current model draft",
        "please publish current model draft",
        "publish the current model draft",
        "publish current model draft",
        "please publish the current model version",
        "please publish current model version",
        "publish the current model version",
        "publish current model version",
        "please publish the current model",
        "please publish current model",
        "publish the current model",
        "publish current model",
        "please publish the current draft",
        "publish the current draft",
        "please publish the current version",
        "publish the current version",
    ];
    let english_description_prefixes = [" with description", ", with description"];
    for command in english_commands {
        if let Some(suffix) = normalized.strip_prefix(command) {
            if terminal_suffix(suffix)
                || english_description_prefixes
                    .iter()
                    .any(|prefix| suffix.starts_with(prefix))
            {
                return true;
            }
            for activation_prefix in [" and activate", ", and activate"] {
                if let Some(tail) = suffix.strip_prefix(activation_prefix) {
                    return terminal_suffix(tail)
                        || english_description_prefixes
                            .iter()
                            .any(|prefix| tail.starts_with(prefix));
                }
            }
            for then_prefix in [", then ", " then "] {
                if let Some(action) = suffix.strip_prefix(then_prefix) {
                    return ["use ", "prepare ", "run "]
                        .iter()
                        .any(|verb| action.starts_with(verb));
                }
            }
            return false;
        }
    }
    false
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCatalogError {
    code: &'static str,
    tool: String,
}

impl ToolCatalogError {
    fn new(code: &'static str, tool: impl Into<String>) -> Self {
        Self {
            code,
            tool: tool.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for ToolCatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.tool)
    }
}

impl std::error::Error for ToolCatalogError {}

#[derive(Clone, Copy)]
struct ToolDefinition {
    name: &'static str,
    build: fn(Arc<ToolServices>) -> RegisteredTool,
}

macro_rules! define_tool_catalog {
    ($( $module:ident :: $tool:ident ),+ $(,)?) => {
        fn tool_definitions() -> &'static [ToolDefinition] {
            &[$(
                ToolDefinition {
                    name: <$module::$tool as Tool>::NAME,
                    build: |services| RegisteredTool::from_tool(
                        support::DomainToolAdapter::new($module::$tool::new(services))
                    ),
                }
            ),+]
        }
    };
}

pub fn tool_implementation_names() -> Vec<&'static str> {
    tool_definitions()
        .iter()
        .map(|definition| definition.name)
        .collect()
}

pub fn validate_tool_catalog(manifest: &AgentManifest) -> Result<(), ToolCatalogError> {
    let mut implementations = BTreeSet::new();
    for definition in tool_definitions() {
        if !implementations.insert(definition.name) {
            return Err(ToolCatalogError::new(
                "agent_tool_implementation_duplicate",
                definition.name,
            ));
        }
    }
    let allowed = manifest
        .allowed_tools
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if let Some(tool) = allowed.difference(&implementations).next() {
        return Err(ToolCatalogError::new(
            "agent_tool_implementation_missing",
            *tool,
        ));
    }
    if let Some(tool) = implementations.difference(&allowed).next() {
        return Err(ToolCatalogError::new(
            "agent_tool_manifest_entry_missing",
            *tool,
        ));
    }
    Ok(())
}

/// 每请求按已经校验的 manifest 顺序装配一套有状态工具（§5）。
pub fn build_tools(services: Arc<ToolServices>) -> Vec<RegisteredTool> {
    validate_tool_catalog(&services.agent_knowledge.manifest)
        .expect("agent tool catalog must match the validated manifest");
    services
        .agent_knowledge
        .manifest
        .allowed_tools
        .iter()
        .map(|name| {
            let definition = tool_definitions()
                .iter()
                .find(|definition| definition.name == name)
                .expect("agent tool catalog was validated during startup");
            (definition.build)(services.clone())
        })
        .collect()
}

define_tool_catalog!(
    inspect_workspace::InspectWorkspaceTool,
    read_model_dataframe::ReadModelDataFrameTool,
    search_study_facades::SearchStudyFacadesTool,
    prepare_study::PrepareStudyTool,
    update_study_results::UpdateStudyResultsTool,
    search_examples::SearchExamplesTool,
    inspect_example::InspectExampleTool,
    modify_model::ModifyModelTool,
    publish_model_version::PublishModelVersionTool,
    list_result_datasets::ListResultDatasetsTool,
    run_result_analysis::RunResultAnalysisTool,
    compare_simulations::CompareSimulationsTool,
    list_simulation_history::ListSimulationHistoryTool,
    search_chat_history::SearchChatHistoryTool,
    read_chat_context::ReadChatContextTool,
    get_storage_usage::GetStorageUsageTool,
    load_agent_skill::LoadAgentSkillTool,
    search_engine_docs::SearchEngineDocsTool,
    read_engine_doc_section::ReadEngineDocSectionTool,
    list_data_resources::ListDataResourcesTool,
    read_data_resource::ReadDataResourceTool,
    update_data_resource_alias::UpdateDataResourceAliasTool,
    promote_result_dataset::PromoteResultDatasetTool,
);
