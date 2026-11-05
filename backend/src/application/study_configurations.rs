use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::CurrentUser;
use crate::catalog::{DataFrameSchemaContract, SchemaFieldContract};
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
use super::study_configuration_policy::{
    apply_result_selector_overrides, json_values_equivalent, preserve_protected_fields,
    result_selector_fields, validate_struct_values,
};
use super::{ColumnMapping, CreateMappedResourceRequest, DataMappingService};
use super::{DataResourceError, DataResourceService};

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
    pub result_selector_fields: Vec<String>,
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
    pub resource_id: Option<String>,
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
pub struct PreparedStructOverlay {
    pub input_id: String,
    pub input_alias: String,
    pub source_resource_id: Option<String>,
    pub schema_ref: String,
    pub effective_values: Value,
    pub user_overrides: Value,
    pub selector_overrides: Value,
    pub overrides: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedStudyConfirmation {
    pub configuration_id: i32,
    pub session_id: i32,
    pub expected_revision: i64,
    pub expected_snapshot_id: String,
    pub struct_overlays: Vec<PreparedStructOverlay>,
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

    pub async fn get(
        &self,
        user: &CurrentUser,
        id: i32,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        self.view(user, self.repo.find(&user.id, id).await?).await
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
                let compatible = self
                    .resources
                    .compatible_for_session(
                        &user.id,
                        aggregate.configuration.session_id,
                        &input.schema_ref,
                    )
                    .await?;
                for resource in fold_selector_only_resources(
                    compatible,
                    &result_selector_fields(&facade, input)?,
                ) {
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
                    result_selector_fields: result_selector_fields(&facade, input)?,
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
}

include!("study_configuration_prepare.rs");
include!("study_configuration_inputs.rs");
include!("study_confirmation.rs");

fn map_struct_resource_error(error: DataResourceError) -> StudyConfigurationError {
    match error {
        DataResourceError::NotFound | DataResourceError::Repository(RepoError::NotFound) => {
            StudyConfigurationError::NotFound
        }
        DataResourceError::Repository(error) => StudyConfigurationError::Repository(error),
        error => {
            let code = error.code();
            StudyConfigurationError::Invalid(code, format!("Struct 资源读取失败: {error}"))
        }
    }
}

fn struct_metadata(baseline: &Value) -> Value {
    serde_json::json!({
        "baselineValues": baseline
    })
}

fn struct_differences(
    binding: &StudyInputBindingRecord,
    current: &Value,
    ignored_fields: &[String],
) -> Value {
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
                if ignored_fields.iter().any(|ignored| ignored == field) {
                    return None;
                }
                let old = before.get(field).cloned().unwrap_or(Value::Null);
                (!json_values_equivalent(&old, value))
                    .then(|| serde_json::json!({"field":field,"before":old,"after":value}))
            })
            .collect(),
    )
}

fn struct_user_overrides(
    baseline: &Value,
    current: &Value,
    ignored_fields: &[String],
) -> Result<Value, StudyConfigurationError> {
    let baseline = baseline.as_object().ok_or_else(|| {
        StudyConfigurationService::invalid("configuration_stale", "Struct 参数基线无效")
    })?;
    let current = current.as_object().ok_or_else(|| {
        StudyConfigurationService::invalid("struct_values_invalid", "Struct 参数必须是对象")
    })?;
    let fields = baseline
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>();
    let mut overrides = serde_json::Map::new();
    for field in fields {
        if ignored_fields.iter().any(|ignored| ignored == field) {
            continue;
        }
        let before = baseline.get(field).unwrap_or(&Value::Null);
        let after = current.get(field).unwrap_or(&Value::Null);
        if !json_values_equivalent(before, after) {
            overrides.insert(field.clone(), after.clone());
        }
    }
    Ok(Value::Object(overrides))
}

fn fold_selector_only_resources(
    resources: Vec<crate::db::models::DataResourceRecord>,
    selector_fields: &[String],
) -> Vec<crate::db::models::DataResourceRecord> {
    if selector_fields.is_empty() {
        return resources;
    }
    let ids = resources
        .iter()
        .map(|resource| resource.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    resources
        .iter()
        .filter(|resource| {
            if resource.source_kind != "struct_edit" {
                return true;
            }
            let Ok(lineage) = serde_json::from_str::<Value>(&resource.lineage_json) else {
                return true;
            };
            let Some(source_id) = lineage.get("replacesResourceId").and_then(Value::as_str) else {
                return true;
            };
            let Some(differences) = lineage.get("diff").and_then(Value::as_array) else {
                return true;
            };
            !(ids.contains(source_id)
                && !differences.is_empty()
                && differences.iter().all(|difference| {
                    difference
                        .get("field")
                        .and_then(Value::as_str)
                        .is_some_and(|field| selector_fields.iter().any(|value| value == field))
                }))
        })
        .cloned()
        .collect()
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
                let selector_fields = result_selector_fields(facade, input)?;
                let values = structs
                    .iter()
                    .find(|value| value.input_name == input.id)
                    .and_then(|value| serde_json::from_str::<Value>(&value.values_json).ok());
                let selected_results = results
                    .iter()
                    .map(|result| result.result_name.clone())
                    .collect::<Vec<_>>();
                let values_valid = values.is_some_and(|mut values| {
                    apply_result_selector_overrides(facade, input, &mut values, &selected_results)
                        .and_then(|_| {
                            validate_struct_values(
                                &service.snapshot,
                                &input.schema_ref,
                                &values,
                                &selector_fields,
                            )
                        })
                        .is_ok()
                });
                let selection_resolved = !binding
                    .mapping_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .and_then(|value| value.get("selectionRequired").and_then(Value::as_bool))
                    .unwrap_or(false);
                values_valid && selection_resolved
            }
        };
        if input.required && !complete {
            return Ok(StudyConfigurationState::CollectingInputs);
        }
    }
    validate_results(facade, results)?;
    Ok(StudyConfigurationState::ReadyToConfirm)
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
            struct_differences(&binding, &serde_json::json!({"base_frequency_hz":50}), &[],),
            serde_json::json!([])
        );
        assert_eq!(
            struct_user_overrides(
                &serde_json::json!({"base_frequency_hz":50.0,"optional":null}),
                &serde_json::json!({"base_frequency_hz":50}),
                &[],
            )
            .unwrap(),
            serde_json::json!({})
        );
    }
}
