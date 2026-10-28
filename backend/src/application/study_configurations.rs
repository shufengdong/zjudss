use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::CurrentUser;
use crate::catalog::{DataFrameSchemaContract, DataTypeContract, SchemaFieldContract};
use crate::db::models::{
    StudyInputBindingRecord, StudyResultSelectionRecord, StudyStructValueRecord,
};
use crate::db::repositories::{
    DataResourceRepo, NewStudyConfiguration, RepoError, StudyConfigurationAggregate,
    StudyConfigurationRepo, VersionRepo, VersionRepoError,
};
use crate::engine_resources::{
    EngineResourceSnapshot, FacadeContract, FacadeInputContract, FacadeInputKind,
    FacadeSearchResult, ResultSelectionRule,
};

use super::now_ms;
use super::{ColumnMapping, CreateMappedResourceRequest, DataMappingService};
use super::{DataResourceService, ReplacementAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyConfigurationState {
    ChoosingFacade,
    CollectingInputs,
    ReadyToConfirm,
    Confirmed,
    Invalidated,
    Cancelled,
}

impl StudyConfigurationState {
    fn parse(value: &str) -> Self {
        match value {
            "choosing_facade" => Self::ChoosingFacade,
            "ready_to_confirm" => Self::ReadyToConfirm,
            "confirmed" => Self::Confirmed,
            "invalidated" => Self::Invalidated,
            "cancelled" => Self::Cancelled,
            _ => Self::CollectingInputs,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ChoosingFacade => "choosing_facade",
            Self::CollectingInputs => "collecting_inputs",
            Self::ReadyToConfirm => "ready_to_confirm",
            Self::Confirmed => "confirmed",
            Self::Invalidated => "invalidated",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareStudyRequest {
    pub facade_id: String,
    pub intent: Value,
    pub derived_from_workflow_id: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectFacadeRequest {
    pub expected_revision: i64,
    pub facade_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindStudyResourceRequest {
    pub expected_revision: i64,
    pub resource_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStudyStructRequest {
    pub expected_revision: i64,
    pub values: Value,
    pub replacement_action: Option<ReplacementAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectStudyResultsRequest {
    pub expected_revision: i64,
    pub results: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapStudyResourceRequest {
    pub expected_revision: i64,
    pub source_resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub mappings: Vec<ColumnMapping>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyResourceOption {
    pub resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub content_hash: String,
    pub row_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyInputBindingView {
    pub model_table_name: Option<String>,
    pub resource_id: Option<String>,
    pub mapping: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructCardView {
    pub schema_ref: String,
    pub fields: Vec<SchemaFieldContract>,
    pub values: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyInputCard {
    pub input_id: String,
    pub alias: String,
    pub kind: String,
    pub schema_ref: String,
    pub required: bool,
    pub position: u32,
    pub binding: StudyInputBindingView,
    pub resource_options: Vec<StudyResourceOption>,
    pub reuse_offer: Option<StudyResourceOption>,
    pub struct_card: Option<StructCardView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyResultCard {
    pub available: Vec<crate::engine_resources::FacadeResultContract>,
    pub selected: Vec<String>,
    pub constraints: Vec<ResultSelectionRule>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyConfigurationView {
    pub id: i32,
    pub session_id: i32,
    pub status: StudyConfigurationState,
    pub revision: i64,
    pub engine_snapshot_id: String,
    pub facade: FacadeContract,
    pub intent: Value,
    pub inputs: Vec<StudyInputCard>,
    pub results: StudyResultCard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalStructValue {
    pub input_id: String,
    pub schema_ref: String,
    pub fields: Value,
    pub resource_id: Option<String>,
    pub resource_content_hash: Option<String>,
    pub resource_diff: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenResultContract {
    pub name: String,
    pub schema_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockedStudyContract {
    pub configuration_id: i32,
    pub facade_id: String,
    pub function: String,
    pub facade_contract_hash: String,
    pub engine_snapshot_id: String,
    pub input_aliases: Vec<String>,
    pub structs: Vec<CanonicalStructValue>,
    pub results: Vec<FrozenResultContract>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenModelInput {
    pub model_version_id: i32,
    pub content_hash: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenStudyInput {
    pub input_id: String,
    pub alias: String,
    pub resource_alias: String,
    pub kind: String,
    pub resource_id: String,
    pub schema_ref: String,
    pub schema_hash: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInputSnapshot {
    pub model: Option<FrozenModelInput>,
    pub resources: Vec<FrozenStudyInput>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedResourceReplacement {
    pub input_id: String,
    pub input_alias: String,
    pub source_resource_id: Option<String>,
    pub action: String,
    pub values: Value,
    pub differences: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedStudyConfirmation {
    pub configuration_id: i32,
    pub expected_revision: i64,
    pub expected_snapshot_id: String,
    pub replacement_actions: Vec<PreparedResourceReplacement>,
    pub locked: LockedStudyContract,
    pub inputs: WorkflowInputSnapshot,
}

#[derive(Debug)]
pub enum StudyConfigurationError {
    NotFound,
    Invalid(&'static str, String),
    Repository(RepoError),
}

impl StudyConfigurationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound | Self::Repository(RepoError::NotFound) => "not_found",
            Self::Invalid(code, _) => code,
            Self::Repository(RepoError::InvalidState(message))
                if message == "configuration_stale" =>
            {
                "configuration_stale"
            }
            _ => "study_configuration_error",
        }
    }
}

impl std::fmt::Display for StudyConfigurationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "研究配置不存在"),
            Self::Invalid(_, message) => write!(formatter, "{message}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for StudyConfigurationError {}

impl From<RepoError> for StudyConfigurationError {
    fn from(value: RepoError) -> Self {
        if matches!(value, RepoError::NotFound) {
            Self::NotFound
        } else {
            Self::Repository(value)
        }
    }
}

#[derive(Clone)]
pub struct StudyConfigurationService {
    repo: StudyConfigurationRepo,
    versions: VersionRepo,
    resources: DataResourceRepo,
    data_resources: DataResourceService,
    data_mapping: DataMappingService,
    snapshot: Arc<EngineResourceSnapshot>,
}

impl StudyConfigurationService {
    pub(crate) fn snapshot_id(&self) -> &str {
        &self.snapshot.id
    }

    pub fn new(
        repo: StudyConfigurationRepo,
        versions: VersionRepo,
        resources: DataResourceRepo,
        data_resources: DataResourceService,
        data_mapping: DataMappingService,
        snapshot: Arc<EngineResourceSnapshot>,
    ) -> Self {
        Self {
            repo,
            versions,
            resources,
            data_resources,
            data_mapping,
            snapshot,
        }
    }

    pub fn search(&self, query: &str, explicit_function: Option<&str>) -> FacadeSearchResult {
        self.snapshot.facades.search(query, explicit_function)
    }

    pub async fn prepare(
        &self,
        user: &CurrentUser,
        session_id: i32,
        request: PrepareStudyRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let facade = self.facade(&request.facade_id)?;
        let (bindings, structs, results, status) =
            self.initial_contents(user, session_id, &facade).await?;
        let intent = serde_json::json!({
            "request": request.intent,
            "derivedFromWorkflowId": request.derived_from_workflow_id,
        });
        let aggregate = self
            .repo
            .create(
                &user.id,
                NewStudyConfiguration {
                    session_id,
                    facade_id: facade.id.clone(),
                    facade_hash: facade.contract_hash.clone(),
                    engine_snapshot_id: self.snapshot.id.clone(),
                    status: status.as_str().into(),
                    intent_json: intent.to_string(),
                    bindings,
                    structs,
                    results,
                    now_ms: now_ms(),
                },
            )
            .await?;
        self.view(user, aggregate).await
    }

    pub async fn get(
        &self,
        user: &CurrentUser,
        id: i32,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        self.view(user, self.repo.find(&user.id, id).await?).await
    }

    pub async fn select_facade(
        &self,
        user: &CurrentUser,
        id: i32,
        request: SelectFacadeRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let current = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&request.facade_id)?;
        let (bindings, structs, results, status) = self
            .initial_contents(user, current.configuration.session_id, &facade)
            .await?;
        self.replace_and_view(
            user,
            current,
            request.expected_revision,
            facade,
            bindings,
            structs,
            results,
            status,
        )
        .await
    }

    pub async fn bind_resource(
        &self,
        user: &CurrentUser,
        id: i32,
        input_id: &str,
        request: BindStudyResourceRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let mut aggregate = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let input = facade
            .inputs
            .iter()
            .find(|input| input.id == input_id)
            .ok_or_else(|| Self::invalid("study_input_not_found", "研究输入不存在"))?;
        if input.kind == FacadeInputKind::Model {
            return Err(Self::invalid(
                "study_binding_invalid",
                "模型输入只能从当前 active 模型动态绑定",
            ));
        }
        if let Some(resource_id) = request.resource_id.as_deref() {
            let resource = self.resources.find(&user.id, resource_id, false).await?;
            if resource.resource.session_id != aggregate.configuration.session_id
                || resource.resource.status != "live"
                || resource.resource.declared_schema_ref.as_deref()
                    != Some(input.schema_ref.as_str())
            {
                return Err(Self::invalid(
                    "study_binding_incompatible",
                    "资源与研究输入 schema 或会话不兼容",
                ));
            }
        }
        let binding = aggregate
            .bindings
            .iter_mut()
            .find(|binding| binding.input_name == input_id)
            .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
        binding.resource_id = request.resource_id;
        binding.mapping_json = None;
        if input.kind == FacadeInputKind::Struct
            && let Some(resource_id) = binding.resource_id.as_deref()
        {
            let values = self
                .struct_resource_values(user, resource_id, input)
                .await?;
            aggregate
                .structs
                .iter_mut()
                .find(|value| value.input_name == input_id)
                .expect("struct inputs always have values")
                .values_json = values.to_string();
            binding.mapping_json = Some(struct_metadata(&values, None).to_string());
        }
        let status = configuration_status(
            &facade,
            &aggregate.bindings,
            &aggregate.structs,
            &aggregate.results,
            self,
        )?;
        self.replace_and_view(
            user,
            aggregate,
            request.expected_revision,
            facade,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            status,
        )
        .await
    }

    pub async fn update_struct(
        &self,
        user: &CurrentUser,
        id: i32,
        input_id: &str,
        request: UpdateStudyStructRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let mut aggregate = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let input = facade
            .inputs
            .iter()
            .find(|input| input.id == input_id && input.kind == FacadeInputKind::Struct)
            .ok_or_else(|| Self::invalid("study_input_not_found", "Struct 输入不存在"))?;
        validate_struct_values(Some(self.schema(&input.schema_ref)?), &request.values)?;
        let binding = aggregate
            .bindings
            .iter_mut()
            .find(|binding| binding.input_name == input_id)
            .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
        let baseline = binding
            .mapping_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .and_then(|value| value.get("baselineValues").cloned());
        let changed = baseline
            .as_ref()
            .is_none_or(|value| value != &request.values);
        if !changed && request.replacement_action.is_some() {
            return Err(Self::invalid(
                "replacement_action_invalid",
                "参数未变化时不能选择覆盖或另存",
            ));
        }
        if binding.resource_id.is_none()
            && matches!(request.replacement_action, Some(ReplacementAction::Replace))
        {
            return Err(Self::invalid(
                "replacement_action_invalid",
                "没有来源资源时只能创建新资源",
            ));
        }
        if request.replacement_action == Some(ReplacementAction::Cancel) {
            return Err(Self::invalid(
                "replacement_action_invalid",
                "取消应保留当前卡片，不作为参数保存动作",
            ));
        }
        binding.mapping_json = Some(
            struct_metadata(
                baseline.as_ref().unwrap_or(&request.values),
                request.replacement_action,
            )
            .to_string(),
        );
        aggregate
            .structs
            .iter_mut()
            .find(|value| value.input_name == input_id)
            .expect("struct inputs always have values")
            .values_json = request.values.to_string();
        let status = configuration_status(
            &facade,
            &aggregate.bindings,
            &aggregate.structs,
            &aggregate.results,
            self,
        )?;
        self.replace_and_view(
            user,
            aggregate,
            request.expected_revision,
            facade,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            status,
        )
        .await
    }

    pub async fn map_resource(
        &self,
        user: &CurrentUser,
        id: i32,
        input_id: &str,
        request: MapStudyResourceRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let aggregate = self.repo.find(&user.id, id).await?;
        if aggregate.configuration.revision != request.expected_revision {
            return Err(Self::invalid(
                "configuration_stale",
                "研究配置 revision 已变化",
            ));
        }
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let input = facade
            .inputs
            .iter()
            .find(|input| input.id == input_id && input.kind == FacadeInputKind::Data)
            .ok_or_else(|| Self::invalid("study_input_not_found", "Data 输入不存在"))?;
        let mapped = self
            .data_mapping
            .create(
                user,
                CreateMappedResourceRequest {
                    session_id: aggregate.configuration.session_id,
                    source_resource_id: request.source_resource_id,
                    display_name: request.display_name,
                    alias: request.alias,
                    target_schema_ref: input.schema_ref.clone(),
                    mappings: request.mappings,
                },
                self.schema(&input.schema_ref)?,
            )
            .await?;
        self.bind_resource(
            user,
            id,
            input_id,
            BindStudyResourceRequest {
                expected_revision: request.expected_revision,
                resource_id: Some(mapped.id),
            },
        )
        .await
    }

    pub async fn select_results(
        &self,
        user: &CurrentUser,
        id: i32,
        request: SelectStudyResultsRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let mut aggregate = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        aggregate.results = request
            .results
            .into_iter()
            .enumerate()
            .map(|(ordinal, result_name)| StudyResultSelectionRecord {
                configuration_id: id,
                result_name,
                ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
            })
            .collect();
        validate_results(&facade, &aggregate.results)?;
        self.validate_conditional_results(user, &facade, &aggregate)
            .await?;
        let status = configuration_status(
            &facade,
            &aggregate.bindings,
            &aggregate.structs,
            &aggregate.results,
            self,
        )?;
        self.replace_and_view(
            user,
            aggregate,
            request.expected_revision,
            facade,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            status,
        )
        .await
    }

    pub async fn cancel(
        &self,
        user: &CurrentUser,
        id: i32,
        expected_revision: i64,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let aggregate = self
            .repo
            .cancel(&user.id, id, expected_revision, now_ms())
            .await?;
        self.view(user, aggregate).await
    }

    pub async fn prepare_confirmation(
        &self,
        user: &CurrentUser,
        configuration_id: i32,
        expected_revision: i64,
    ) -> Result<PreparedStudyConfirmation, StudyConfigurationError> {
        let aggregate = self.repo.find(&user.id, configuration_id).await?;
        if aggregate.configuration.revision != expected_revision
            || aggregate.configuration.engine_snapshot_id != self.snapshot.id
            || aggregate.configuration.status == "invalidated"
        {
            return Err(Self::invalid(
                "configuration_stale",
                "研究配置、资源或引擎快照已变化",
            ));
        }
        if aggregate.configuration.status != "ready_to_confirm" {
            return Err(Self::invalid(
                "configuration_incomplete",
                "研究函数、输入、参数或结果尚未全部确认",
            ));
        }
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        self.validate_conditional_results(user, &facade, &aggregate)
            .await?;
        if facade.contract_hash != aggregate.configuration.facade_hash {
            return Err(Self::invalid("configuration_stale", "研究门面合同已经变化"));
        }
        let workspace = self
            .versions
            .workspace(&user.id, aggregate.configuration.session_id, now_ms())
            .await
            .map_err(map_version_error)?;
        let model_bindings = aggregate
            .bindings
            .iter()
            .filter(|binding| binding.input_kind == "model")
            .collect::<Vec<_>>();
        let model = if model_bindings.is_empty() {
            None
        } else {
            let active = workspace.active.ok_or_else(|| {
                Self::invalid("configuration_stale", "当前 active 模型已变化或不存在")
            })?;
            let expected = model_bindings
                .iter()
                .filter_map(|binding| binding.model_table_name.clone())
                .collect::<Vec<_>>();
            if expected.len() != model_bindings.len()
                || expected.iter().any(|name| {
                    !active
                        .dataframes
                        .iter()
                        .any(|frame| frame.table_name == *name)
                })
            {
                return Err(Self::invalid(
                    "configuration_stale",
                    "当前 active 模型不再满足研究门面",
                ));
            }
            Some(FrozenModelInput {
                model_version_id: active.version.id,
                content_hash: active.version.content_hash,
                tables: expected,
            })
        };
        let mut frozen_resources = Vec::new();
        let mut structs = Vec::new();
        let mut replacements = Vec::new();
        let selected_result_names = aggregate
            .results
            .iter()
            .map(|result| result.result_name.clone())
            .collect::<Vec<_>>();
        for input in &facade.inputs {
            let binding = aggregate
                .bindings
                .iter()
                .find(|binding| binding.input_name == input.id)
                .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
            if let Some(resource_id) = binding.resource_id.as_deref() {
                let resource = self.resources.find(&user.id, resource_id, false).await?;
                if resource.resource.status != "live"
                    || resource.resource.declared_schema_ref.as_deref()
                        != Some(input.schema_ref.as_str())
                {
                    return Err(Self::invalid(
                        "configuration_stale",
                        "绑定资源已经变化或不再兼容",
                    ));
                }
                frozen_resources.push(FrozenStudyInput {
                    input_id: input.id.clone(),
                    alias: input.alias.clone(),
                    resource_alias: resource.resource.alias,
                    kind: kind_name(input.kind).into(),
                    resource_id: resource.resource.id,
                    schema_ref: input.schema_ref.clone(),
                    schema_hash: resource.resource.schema_hash,
                    content_hash: resource.normalized_sha256.clone(),
                });
            }
            if input.kind == FacadeInputKind::Struct {
                let value = aggregate
                    .structs
                    .iter()
                    .find(|value| value.input_name == input.id)
                    .ok_or_else(|| Self::invalid("configuration_stale", "Struct 参数缺失"))?;
                let mut fields: Value = serde_json::from_str(&value.values_json).map_err(|_| {
                    Self::invalid("configuration_stale", "Struct 参数持久化内容无效")
                })?;
                let selector_changed =
                    apply_result_selectors(&facade, input, &mut fields, &selected_result_names)?;
                validate_struct_values(Some(self.schema(&input.schema_ref)?), &fields)?;
                let frozen = frozen_resources
                    .iter()
                    .find(|resource| resource.input_id == input.id);
                structs.push(CanonicalStructValue {
                    input_id: input.id.clone(),
                    schema_ref: input.schema_ref.clone(),
                    fields: fields.clone(),
                    resource_id: frozen.map(|resource| resource.resource_id.clone()),
                    resource_content_hash: frozen.map(|resource| resource.content_hash.clone()),
                    resource_diff: struct_differences(binding, &fields),
                });
                let action = struct_action(binding);
                if frozen.is_none() || action.is_some() || selector_changed {
                    replacements.push(PreparedResourceReplacement {
                        input_id: input.id.clone(),
                        input_alias: input.alias.clone(),
                        source_resource_id: frozen.map(|resource| resource.resource_id.clone()),
                        action: action.unwrap_or("create_new").into(),
                        values: fields,
                        differences: match &structs.last().expect("just pushed").resource_diff {
                            Value::Array(values) => values.clone(),
                            _ => Vec::new(),
                        },
                    });
                }
            }
        }
        let results = aggregate
            .results
            .iter()
            .map(|selected| {
                let result = facade
                    .results
                    .iter()
                    .find(|result| result.name == selected.result_name)
                    .expect("stored results validated against facade");
                FrozenResultContract {
                    name: result.name.clone(),
                    schema_ref: result.schema_ref.clone(),
                }
            })
            .collect();
        Ok(PreparedStudyConfirmation {
            configuration_id,
            expected_revision,
            expected_snapshot_id: self.snapshot.id.clone(),
            replacement_actions: replacements,
            locked: LockedStudyContract {
                configuration_id,
                facade_id: facade.id,
                function: facade.function,
                facade_contract_hash: facade.contract_hash,
                engine_snapshot_id: self.snapshot.id.clone(),
                input_aliases: facade
                    .inputs
                    .iter()
                    .map(|input| input.alias.clone())
                    .collect(),
                structs,
                results,
            },
            inputs: WorkflowInputSnapshot {
                model,
                resources: frozen_resources,
            },
        })
    }

    async fn initial_contents(
        &self,
        user: &CurrentUser,
        session_id: i32,
        facade: &FacadeContract,
    ) -> Result<
        (
            Vec<StudyInputBindingRecord>,
            Vec<StudyStructValueRecord>,
            Vec<StudyResultSelectionRecord>,
            StudyConfigurationState,
        ),
        StudyConfigurationError,
    > {
        let workspace = self
            .versions
            .workspace(&user.id, session_id, now_ms())
            .await
            .map_err(map_version_error)?;
        let mut bindings = Vec::new();
        let mut structs = Vec::new();
        for input in &facade.inputs {
            let mut model_table_name = None;
            let mut resource_id = None;
            let mut mapping_json = None;
            match input.kind {
                FacadeInputKind::Model => {
                    model_table_name = workspace.active.as_ref().and_then(|active| {
                        active
                            .dataframes
                            .iter()
                            .find(|frame| frame.table_name == input.alias)
                            .map(|frame| frame.table_name.clone())
                    });
                }
                FacadeInputKind::Data | FacadeInputKind::Struct => {
                    let compatible = self
                        .resources
                        .compatible_for_session(&user.id, session_id, &input.schema_ref)
                        .await?;
                    let prior = if input.kind == FacadeInputKind::Struct {
                        self.repo
                            .latest_confirmed_resource(&user.id, session_id, &facade.id, &input.id)
                            .await?
                    } else {
                        None
                    };
                    let has_live_prior = prior
                        .as_deref()
                        .is_some_and(|id| compatible.iter().any(|resource| resource.id == id));
                    if compatible.len() == 1 && !has_live_prior {
                        resource_id = Some(compatible[0].id.clone());
                    } else if compatible.len() > 1 || has_live_prior {
                        mapping_json =
                            Some(serde_json::json!({"selectionRequired":true}).to_string());
                    }
                }
            }
            bindings.push(StudyInputBindingRecord {
                configuration_id: 0,
                input_name: input.id.clone(),
                input_kind: kind_name(input.kind).into(),
                model_table_name,
                resource_id: resource_id.clone(),
                mapping_json: mapping_json.take(),
                ordinal: i32::try_from(input.position).unwrap_or(i32::MAX),
            });
            if input.kind == FacadeInputKind::Struct {
                let schema = self.schema(&input.schema_ref)?;
                let mut values = struct_defaults(schema);
                if let Some(selected) = resource_id.as_deref() {
                    values = self.struct_resource_values(user, selected, input).await?;
                    if let Some(binding) = bindings.last_mut() {
                        binding.mapping_json = Some(struct_metadata(&values, None).to_string());
                    }
                }
                structs.push(StudyStructValueRecord {
                    configuration_id: 0,
                    input_name: input.id.clone(),
                    values_json: values.to_string(),
                    schema_ref: input.schema_ref.clone(),
                });
            }
        }
        let selected = default_results(facade);
        let results = selected
            .into_iter()
            .enumerate()
            .map(|(ordinal, name)| StudyResultSelectionRecord {
                configuration_id: 0,
                result_name: name,
                ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
            })
            .collect::<Vec<_>>();
        let status = configuration_status(facade, &bindings, &structs, &results, self)?;
        Ok((bindings, structs, results, status))
    }

    async fn view(
        &self,
        user: &CurrentUser,
        aggregate: StudyConfigurationAggregate,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let mut inputs = Vec::new();
        for input in &facade.inputs {
            let binding = aggregate
                .bindings
                .iter()
                .find(|binding| binding.input_name == input.id)
                .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
            let mut resource_options = Vec::new();
            if input.kind != FacadeInputKind::Model {
                for resource in self
                    .resources
                    .compatible_for_session(
                        &user.id,
                        aggregate.configuration.session_id,
                        &input.schema_ref,
                    )
                    .await?
                {
                    let row = self.resources.find(&user.id, &resource.id, false).await?;
                    resource_options.push(StudyResourceOption {
                        resource_id: resource.id,
                        display_name: resource.display_name,
                        alias: resource.alias,
                        content_hash: row.normalized_sha256,
                        row_count: resource.row_count,
                    });
                }
            }
            let struct_card = if input.kind == FacadeInputKind::Struct {
                let stored = aggregate
                    .structs
                    .iter()
                    .find(|value| value.input_name == input.id)
                    .ok_or_else(|| Self::invalid("configuration_stale", "Struct 参数缺失"))?;
                Some(StructCardView {
                    schema_ref: input.schema_ref.clone(),
                    fields: self.schema(&input.schema_ref)?.fields.clone(),
                    values: serde_json::from_str(&stored.values_json).map_err(|_| {
                        Self::invalid("configuration_stale", "Struct 参数持久化内容无效")
                    })?,
                })
            } else {
                None
            };
            let reuse_offer = if input.kind == FacadeInputKind::Struct {
                let prior = self
                    .repo
                    .latest_confirmed_resource(
                        &user.id,
                        aggregate.configuration.session_id,
                        &facade.id,
                        &input.id,
                    )
                    .await?;
                prior.and_then(|id| {
                    resource_options
                        .iter()
                        .find(|option| option.resource_id == id)
                        .cloned()
                })
            } else {
                None
            };
            inputs.push(StudyInputCard {
                input_id: input.id.clone(),
                alias: input.alias.clone(),
                kind: kind_name(input.kind).into(),
                schema_ref: input.schema_ref.clone(),
                required: input.required,
                position: input.position,
                binding: StudyInputBindingView {
                    model_table_name: binding.model_table_name.clone(),
                    resource_id: binding.resource_id.clone(),
                    mapping: binding
                        .mapping_json
                        .as_deref()
                        .and_then(|value| serde_json::from_str(value).ok()),
                },
                resource_options,
                reuse_offer,
                struct_card,
            });
        }
        Ok(StudyConfigurationView {
            id: aggregate.configuration.id,
            session_id: aggregate.configuration.session_id,
            status: StudyConfigurationState::parse(&aggregate.configuration.status),
            revision: aggregate.configuration.revision,
            engine_snapshot_id: aggregate.configuration.engine_snapshot_id,
            facade: facade.clone(),
            intent: serde_json::from_str(&aggregate.configuration.intent_json)
                .unwrap_or(Value::Null),
            inputs,
            results: StudyResultCard {
                available: facade.results.clone(),
                selected: aggregate
                    .results
                    .iter()
                    .map(|result| result.result_name.clone())
                    .collect(),
                constraints: facade.result_constraints.clone(),
            },
        })
    }

    fn facade(&self, id: &str) -> Result<FacadeContract, StudyConfigurationError> {
        self.snapshot
            .facades
            .get_enabled(id)
            .cloned()
            .ok_or_else(|| Self::invalid("study_facade_unavailable", "研究门面不存在或已禁用"))
    }

    fn schema(
        &self,
        schema_ref: &str,
    ) -> Result<&DataFrameSchemaContract, StudyConfigurationError> {
        self.snapshot
            .schemas
            .resolve(schema_ref)
            .map_err(|error| Self::invalid("study_schema_unavailable", &error.to_string()))
    }

    fn invalid(code: &'static str, message: &str) -> StudyConfigurationError {
        StudyConfigurationError::Invalid(code, message.into())
    }

    async fn struct_resource_values(
        &self,
        user: &CurrentUser,
        resource_id: &str,
        input: &FacadeInputContract,
    ) -> Result<Value, StudyConfigurationError> {
        let page = self
            .data_resources
            .rows(user, resource_id, Some(2), None, Vec::new())
            .await
            .map_err(|error| Self::invalid("study_binding_incompatible", &error.to_string()))?;
        if page.rows.is_empty() {
            return Ok(struct_defaults(self.schema(&input.schema_ref)?));
        }
        if page.rows.len() != 1 {
            return Err(Self::invalid(
                "study_binding_incompatible",
                "Struct 资源必须恰好包含一行",
            ));
        }
        let values = page
            .columns
            .iter()
            .cloned()
            .zip(page.rows[0].iter().cloned())
            .collect::<serde_json::Map<_, _>>();
        let values = Value::Object(values);
        validate_struct_values(Some(self.schema(&input.schema_ref)?), &values)?;
        Ok(values)
    }

    async fn validate_conditional_results(
        &self,
        user: &CurrentUser,
        facade: &FacadeContract,
        aggregate: &StudyConfigurationAggregate,
    ) -> Result<(), StudyConfigurationError> {
        for constraint in &facade.result_constraints {
            let ResultSelectionRule::ConditionalUnavailable { result, when } = constraint else {
                continue;
            };
            if !aggregate
                .results
                .iter()
                .any(|selected| selected.result_name == *result)
            {
                continue;
            }
            let binding = aggregate
                .bindings
                .iter()
                .find(|binding| binding.input_name == when.input);
            let non_empty = if let Some(resource_id) =
                binding.and_then(|binding| binding.resource_id.as_deref())
            {
                self.resources
                    .find(&user.id, resource_id, false)
                    .await?
                    .resource
                    .row_count
                    > 0
            } else {
                false
            };
            if non_empty == when.non_empty {
                return Err(Self::invalid(
                    "result_selection_invalid",
                    "当前输入条件下该结果不可用",
                ));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn replace_and_view(
        &self,
        user: &CurrentUser,
        aggregate: StudyConfigurationAggregate,
        expected_revision: i64,
        facade: FacadeContract,
        bindings: Vec<StudyInputBindingRecord>,
        structs: Vec<StudyStructValueRecord>,
        results: Vec<StudyResultSelectionRecord>,
        status: StudyConfigurationState,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let bindings = if bindings.is_empty() {
            aggregate.bindings
        } else {
            bindings
        };
        let structs = if structs.is_empty() {
            aggregate.structs
        } else {
            structs
        };
        let results = if results.is_empty() {
            aggregate.results
        } else {
            results
        };
        let updated = self
            .repo
            .replace(
                &user.id,
                aggregate.configuration.id,
                expected_revision,
                facade.id.clone(),
                facade.contract_hash.clone(),
                self.snapshot.id.clone(),
                status.as_str().into(),
                bindings,
                structs,
                results,
                now_ms(),
            )
            .await?;
        self.view(user, updated).await
    }
}

fn struct_metadata(baseline: &Value, action: Option<ReplacementAction>) -> Value {
    serde_json::json!({
        "baselineValues": baseline,
        "replacementAction": action.map(|value| match value {
            ReplacementAction::Replace => "replace",
            ReplacementAction::SaveAsNew => "save_as_new",
            ReplacementAction::Cancel => "cancel",
        })
    })
}

fn struct_action(binding: &StudyInputBindingRecord) -> Option<&str> {
    binding
        .mapping_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| value.get("replacementAction")?.as_str().map(str::to_string))
        .and_then(|value| match value.as_str() {
            "replace" => Some("replace"),
            "save_as_new" => Some("save_as_new"),
            _ => None,
        })
}

fn struct_differences(binding: &StudyInputBindingRecord, current: &Value) -> Value {
    let baseline = binding
        .mapping_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .and_then(|value| value.get("baselineValues").cloned());
    let Some((before, after)) = baseline
        .as_ref()
        .and_then(Value::as_object)
        .zip(current.as_object())
    else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        after
            .iter()
            .filter_map(|(field, value)| {
                let old = before.get(field).cloned().unwrap_or(Value::Null);
                (!json_values_equivalent(&old, value))
                    .then(|| serde_json::json!({"field":field,"before":old,"after":value}))
            })
            .collect(),
    )
}

fn json_values_equivalent(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    let (Some(left), Some(right)) = (left.as_number(), right.as_number()) else {
        return false;
    };
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return left == right;
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return left == right;
    }
    left.as_f64()
        .zip(right.as_f64())
        .is_some_and(|(left, right)| left == right)
}

fn apply_result_selectors(
    facade: &FacadeContract,
    input: &FacadeInputContract,
    fields: &mut Value,
    selected_results: &[String],
) -> Result<bool, StudyConfigurationError> {
    let object = fields.as_object_mut().ok_or_else(|| {
        StudyConfigurationService::invalid("struct_values_invalid", "Struct 参数必须是对象")
    })?;
    let mut changed = false;
    for constraint in &facade.result_constraints {
        let ResultSelectionRule::OrderedUniqueSelection { selector, .. } = constraint else {
            continue;
        };
        let Some((selector_input, field)) = selector.split_once('.') else {
            return Err(StudyConfigurationService::invalid(
                "configuration_stale",
                "结果选择器格式无效",
            ));
        };
        if selector_input != input.alias && selector_input != input.id {
            continue;
        }
        let value = Value::String(selected_results.join(","));
        if object.get(field) != Some(&value) {
            object.insert(field.to_string(), value);
            changed = true;
        }
    }
    Ok(changed)
}

fn map_version_error(error: VersionRepoError) -> StudyConfigurationError {
    match error {
        VersionRepoError::Repository(RepoError::NotFound) => StudyConfigurationError::NotFound,
        VersionRepoError::Repository(error) => StudyConfigurationError::Repository(error),
        _ => StudyConfigurationService::invalid(
            "configuration_stale",
            "模型工作区状态不允许准备研究",
        ),
    }
}

fn kind_name(kind: FacadeInputKind) -> &'static str {
    match kind {
        FacadeInputKind::Model => "model",
        FacadeInputKind::Struct => "struct",
        FacadeInputKind::Data => "data",
    }
}

fn struct_defaults(schema: &DataFrameSchemaContract) -> Value {
    let mut values = serde_json::Map::new();
    for field in &schema.fields {
        if let Some(value) = field
            .card
            .as_ref()
            .filter(|card| card.default.state == "value")
            .and_then(|card| card.default.value.clone())
        {
            values.insert(field.name.clone(), value);
        }
    }
    Value::Object(values)
}

fn default_results(facade: &FacadeContract) -> Vec<String> {
    for constraint in &facade.result_constraints {
        if let ResultSelectionRule::FixedResults { results, .. }
        | ResultSelectionRule::DefaultResults { results } = constraint
        {
            return results.clone();
        }
    }
    facade
        .results
        .iter()
        .filter(|result| result.recommended)
        .map(|result| result.name.clone())
        .collect()
}

fn configuration_status(
    facade: &FacadeContract,
    bindings: &[StudyInputBindingRecord],
    structs: &[StudyStructValueRecord],
    results: &[StudyResultSelectionRecord],
    service: &StudyConfigurationService,
) -> Result<StudyConfigurationState, StudyConfigurationError> {
    for input in &facade.inputs {
        let binding = bindings
            .iter()
            .find(|binding| binding.input_name == input.id)
            .ok_or_else(|| {
                StudyConfigurationService::invalid("configuration_stale", "研究输入绑定缺失")
            })?;
        let complete = match input.kind {
            FacadeInputKind::Model => binding.model_table_name.is_some(),
            FacadeInputKind::Data => binding.resource_id.is_some() || !input.required,
            FacadeInputKind::Struct => {
                let values = structs
                    .iter()
                    .find(|value| value.input_name == input.id)
                    .and_then(|value| serde_json::from_str::<Value>(&value.values_json).ok());
                let values_valid = values.is_some_and(|values| {
                    validate_struct_values(service.schema(&input.schema_ref).ok(), &values).is_ok()
                });
                let selection_resolved = !binding
                    .mapping_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .and_then(|value| value.get("selectionRequired").and_then(Value::as_bool))
                    .unwrap_or(false);
                let replacement_selected = binding.resource_id.is_none()
                    || struct_differences(
                        binding,
                        &serde_json::from_str::<Value>(
                            &structs
                                .iter()
                                .find(|value| value.input_name == input.id)
                                .expect("struct input has values")
                                .values_json,
                        )
                        .unwrap_or(Value::Null),
                    )
                    .as_array()
                    .is_none_or(Vec::is_empty)
                    || struct_action(binding).is_some();
                values_valid && selection_resolved && replacement_selected
            }
        };
        if input.required && !complete {
            return Ok(StudyConfigurationState::CollectingInputs);
        }
    }
    validate_results(facade, results)?;
    Ok(StudyConfigurationState::ReadyToConfirm)
}

fn validate_struct_values(
    schema: Option<&DataFrameSchemaContract>,
    values: &Value,
) -> Result<(), StudyConfigurationError> {
    let schema = schema.ok_or_else(|| {
        StudyConfigurationService::invalid("study_schema_unavailable", "Struct schema 不可用")
    })?;
    let object = values.as_object().ok_or_else(|| {
        StudyConfigurationService::invalid("struct_values_invalid", "Struct 参数必须是对象")
    })?;
    let known = schema
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if object.keys().any(|name| !known.contains(name.as_str())) {
        return Err(StudyConfigurationService::invalid(
            "struct_values_invalid",
            "Struct 参数包含未知字段",
        ));
    }
    for field in &schema.fields {
        let value = object.get(&field.name);
        if field.required && value.is_none() {
            return Err(StudyConfigurationService::invalid(
                "struct_values_invalid",
                "Struct 参数缺少必填字段",
            ));
        }
        if let Some(value) = value {
            validate_field(field, value)?;
        }
    }
    Ok(())
}

fn validate_field(
    field: &SchemaFieldContract,
    value: &Value,
) -> Result<(), StudyConfigurationError> {
    if value.is_null() && field.nullable {
        return Ok(());
    }
    let valid_type = match &field.dtype {
        DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => {
            match name.as_str() {
                "str" | "string" => value.is_string(),
                "bool" => value.is_boolean(),
                name if name.starts_with('u') || name.starts_with('i') => {
                    value.as_i64().is_some() || value.as_u64().is_some()
                }
                _ => value.is_number(),
            }
        }
        DataTypeContract::Constructor { .. } => value.is_array(),
    };
    if !valid_type {
        return Err(StudyConfigurationService::invalid(
            "struct_values_invalid",
            "Struct 参数类型不匹配",
        ));
    }
    if let Some(card) = &field.card {
        if card.enum_values.state == "values"
            && !card.enum_values.values.iter().any(|candidate| {
                candidate == value
                    || (card.enum_values.case_insensitive
                        && candidate
                            .as_str()
                            .zip(value.as_str())
                            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right)))
            })
        {
            return Err(StudyConfigurationService::invalid(
                "struct_values_invalid",
                "Struct 参数不在允许枚举中",
            ));
        }
        if card.range.state == "bounded" {
            let number = value.as_f64().ok_or_else(|| {
                StudyConfigurationService::invalid("struct_values_invalid", "范围约束要求数值")
            })?;
            if let Some(minimum) = card.range.minimum.as_ref().and_then(Value::as_f64) {
                let inclusive = card.range.minimum_inclusive.unwrap_or(true);
                if number < minimum || (!inclusive && number == minimum) {
                    return Err(StudyConfigurationService::invalid(
                        "struct_values_invalid",
                        "Struct 参数低于最小值",
                    ));
                }
            }
            if let Some(maximum) = card.range.maximum.as_ref().and_then(Value::as_f64) {
                let inclusive = card.range.maximum_inclusive.unwrap_or(true);
                if number > maximum || (!inclusive && number == maximum) {
                    return Err(StudyConfigurationService::invalid(
                        "struct_values_invalid",
                        "Struct 参数高于最大值",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_results(
    facade: &FacadeContract,
    results: &[StudyResultSelectionRecord],
) -> Result<(), StudyConfigurationError> {
    let selected = results
        .iter()
        .map(|result| result.result_name.as_str())
        .collect::<Vec<_>>();
    let unique = selected.iter().copied().collect::<BTreeSet<_>>();
    if selected.is_empty()
        || unique.len() != selected.len()
        || selected
            .iter()
            .any(|name| !facade.results.iter().any(|result| result.name == *name))
    {
        return Err(StudyConfigurationService::invalid(
            "result_selection_invalid",
            "结果选择为空、重复或包含未知结果",
        ));
    }
    for constraint in &facade.result_constraints {
        match constraint {
            ResultSelectionRule::FixedResults { results, .. }
                if results.iter().map(String::as_str).collect::<Vec<_>>() != selected =>
            {
                return Err(StudyConfigurationService::invalid(
                    "result_selection_invalid",
                    "固定结果集合不能修改",
                ));
            }
            ResultSelectionRule::OrderedUniqueSelection { minimum, .. }
                if selected.len() < *minimum as usize =>
            {
                return Err(StudyConfigurationService::invalid(
                    "result_selection_invalid",
                    "结果数量不满足门面约束",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_differences_treats_equivalent_json_numbers_as_equal() {
        let binding = StudyInputBindingRecord {
            configuration_id: 1,
            input_name: "para".into(),
            input_kind: "struct".into(),
            model_table_name: None,
            resource_id: Some("resource-1".into()),
            mapping_json: Some(
                serde_json::json!({"baselineValues":{"base_frequency_hz":50.0}}).to_string(),
            ),
            ordinal: 1,
        };

        assert_eq!(
            struct_differences(&binding, &serde_json::json!({"base_frequency_hz":50})),
            serde_json::json!([])
        );
    }
}
