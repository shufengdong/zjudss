use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::catalog::{CatalogError, DataFrameContractKind, SchemaRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacadeInputKind {
    Model,
    Struct,
    Data,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacadeInputContract {
    pub id: String,
    pub alias: String,
    pub kind: FacadeInputKind,
    pub schema_ref: String,
    pub required: bool,
    pub position: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacadeResultContract {
    pub id: String,
    pub name: String,
    pub schema_ref: String,
    pub description: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ResultSelectionRule {
    OrderedUniqueSelection {
        selector: String,
        minimum: u32,
        binding_count: String,
    },
    DefaultResults {
        results: Vec<String>,
    },
    FixedResults {
        results: Vec<String>,
        binding_count: u32,
    },
    ConditionalUnavailable {
        result: String,
        when: ResultCondition,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultCondition {
    pub input: String,
    pub non_empty: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FacadeContract {
    pub id: String,
    pub function: String,
    pub family: String,
    pub title: String,
    pub description: String,
    pub differences: Vec<String>,
    pub inputs: Vec<FacadeInputContract>,
    pub results: Vec<FacadeResultContract>,
    pub result_constraints: Vec<ResultSelectionRule>,
    pub contract_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisabledFacadeDiagnostic {
    pub facade_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FacadeSearchResult {
    pub enabled: Vec<FacadeContract>,
    pub disabled: Vec<DisabledFacadeDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct FacadeRegistry {
    catalog_entry_count: usize,
    enabled: BTreeMap<String, FacadeContract>,
    disabled: BTreeMap<String, DisabledFacadeDiagnostic>,
}

impl FacadeRegistry {
    pub(crate) fn unavailable(code: &str, message: &str) -> Self {
        Self {
            catalog_entry_count: 0,
            enabled: BTreeMap::new(),
            disabled: BTreeMap::from([(
                "_catalog".to_string(),
                diagnostic("_catalog", code, message),
            )]),
        }
    }

    pub(crate) fn from_catalogs(
        facades_root: &Value,
        schemas: &mut SchemaRegistry,
    ) -> Result<Self, CatalogError> {
        let entries = facades_root
            .get("facades")
            .and_then(Value::as_array)
            .ok_or_else(|| CatalogError::new("catalog_invalid", "study_facades 缺少 facades"))?;
        let mut enabled = BTreeMap::new();
        let mut disabled = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let facade_id = entry
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("catalog-entry-{index}"));
            match parse_facade(entry, schemas) {
                Ok(facade) => {
                    if enabled.contains_key(&facade.id) || disabled.contains_key(&facade.id) {
                        disabled.insert(
                            facade.id.clone(),
                            diagnostic(&facade.id, "facade_duplicate_id", "研究门面 id 重复"),
                        );
                        enabled.remove(&facade.id);
                    } else {
                        enabled.insert(facade.id.clone(), facade);
                    }
                }
                Err((code, message)) => {
                    let function = entry
                        .get("function")
                        .and_then(Value::as_str)
                        .unwrap_or("<unknown>");
                    disabled.insert(
                        facade_id.clone(),
                        diagnostic(&facade_id, code, format!("function={function}: {message}")),
                    );
                }
            }
        }
        Ok(Self {
            catalog_entry_count: entries.len(),
            enabled,
            disabled,
        })
    }

    pub fn catalog_entry_count(&self) -> usize {
        self.catalog_entry_count
    }

    pub fn enabled_count(&self) -> usize {
        self.enabled.len()
    }

    pub fn diagnostics(&self) -> Vec<&DisabledFacadeDiagnostic> {
        self.disabled.values().collect()
    }

    pub fn diagnostic(&self, facade_id: &str) -> Option<&DisabledFacadeDiagnostic> {
        self.disabled.get(facade_id)
    }

    pub fn get_enabled(&self, facade_id: &str) -> Option<&FacadeContract> {
        self.enabled.get(facade_id)
    }

    pub fn enabled(&self) -> impl Iterator<Item = &FacadeContract> {
        self.enabled.values()
    }

    pub fn search(&self, query: &str, explicit_function: Option<&str>) -> FacadeSearchResult {
        if let Some(function) = explicit_function.filter(|value| !value.trim().is_empty()) {
            let function = function.trim();
            let enabled = self
                .enabled
                .values()
                .filter(|facade| facade.function.eq_ignore_ascii_case(function))
                .cloned()
                .collect();
            let disabled = self
                .disabled
                .values()
                .filter(|diagnostic| {
                    diagnostic.facade_id.eq_ignore_ascii_case(function)
                        || diagnostic.message.contains(function)
                })
                .cloned()
                .collect();
            return FacadeSearchResult { enabled, disabled };
        }
        let terms = query
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .collect::<Vec<_>>();
        let scored = self
            .enabled
            .values()
            .filter_map(|facade| {
                if terms.is_empty() {
                    return Some((facade, 0));
                }
                let core = format!(
                    "{} {} {} {} {}",
                    facade.id, facade.function, facade.family, facade.title, facade.description
                )
                .to_lowercase();
                let differences = facade.differences.join(" ").to_lowercase();
                let score = terms.iter().fold(0_u32, |score, term| {
                    score
                        + if core.contains(term) {
                            2
                        } else if differences.contains(term) {
                            1
                        } else {
                            0
                        }
                });
                (score > 0).then_some((facade, score))
            })
            .collect::<Vec<_>>();
        let highest_score = scored.iter().map(|(_, score)| *score).max().unwrap_or(0);
        let enabled = scored
            .into_iter()
            .filter(|(_, score)| *score == highest_score)
            .map(|(facade, _)| facade.clone())
            .collect();
        FacadeSearchResult {
            enabled,
            disabled: Vec::new(),
        }
    }
}

fn parse_facade(
    value: &Value,
    schemas: &mut SchemaRegistry,
) -> Result<FacadeContract, (&'static str, String)> {
    let value = value
        .as_object()
        .ok_or_else(|| invalid("facade 不是对象"))?;
    let id = required_string(value, "id")?;
    let function = required_string(value, "function")?;
    if function.starts_with("import_") || function.ends_with("_url") || function.contains("path") {
        return Err((
            "facade_unsupported_input_kind",
            format!("{id} 是路径、URL 或导入门面，不是 DataFrame 研究门面"),
        ));
    }
    let family = required_string(value, "family")?;
    let title = required_string(value, "title")?;
    let description = required_string(value, "description")?;
    let differences = required_string_array(value, "differences")?;
    if differences.is_empty() {
        return Err(invalid("differences 不能为空"));
    }
    let inputs_value = required_array(value, "inputs")?;
    let mut inputs = Vec::with_capacity(inputs_value.len());
    let mut input_schemas = Vec::with_capacity(inputs_value.len());
    let mut positions = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    for input in inputs_value {
        let input = input
            .as_object()
            .ok_or_else(|| invalid("inputs 包含非对象"))?;
        let input_id = required_string(input, "id")?;
        let alias = required_string(input, "alias")?;
        let kind = match required_string(input, "inputKind")?.as_str() {
            "model" => FacadeInputKind::Model,
            "struct" => FacadeInputKind::Struct,
            "data" => FacadeInputKind::Data,
            other => {
                return Err((
                    "facade_unsupported_input_kind",
                    format!("{id}.{input_id} 使用不支持的 inputKind: {other}"),
                ));
            }
        };
        let schema_ref = required_string(input, "schemaRef")?;
        let required = required_bool(input, "required")?;
        let position = required_u32(input, "position")?;
        if position == 0 || !positions.insert(position) || !aliases.insert(alias.clone()) {
            return Err(invalid("输入 position 或 alias 重复/无效"));
        }
        let schema = schemas
            .resolve_for_facade(&schema_ref)
            .map_err(|error| ("facade_schema_invalid", format!("{id}.{input_id}: {error}")))?;
        let expected_kind = match kind {
            FacadeInputKind::Model => DataFrameContractKind::Model,
            FacadeInputKind::Struct => DataFrameContractKind::Struct,
            FacadeInputKind::Data => DataFrameContractKind::Data,
        };
        if schema.kind != expected_kind {
            return Err((
                "facade_schema_invalid",
                format!("{id}.{input_id} 的 inputKind 与 schema kind 不一致"),
            ));
        }
        input_schemas.push(schema);
        inputs.push(FacadeInputContract {
            id: input_id,
            alias,
            kind,
            schema_ref,
            required,
            position,
        });
    }
    if inputs.is_empty() {
        return Err(invalid("inputs 不能为空"));
    }
    inputs.sort_by_key(|input| input.position);
    if inputs
        .iter()
        .enumerate()
        .any(|(index, input)| input.position != (index + 1) as u32)
    {
        return Err(invalid("输入 position 必须从 1 连续排列"));
    }

    let results_value = required_array(value, "results")?;
    let mut results = Vec::with_capacity(results_value.len());
    let mut result_schemas = Vec::with_capacity(results_value.len());
    let mut result_names = BTreeSet::new();
    for result in results_value {
        let result = result
            .as_object()
            .ok_or_else(|| invalid("results 包含非对象"))?;
        let result_id = required_string(result, "id")?;
        let name = required_string(result, "name")?;
        if !result_names.insert(name.clone()) {
            return Err(invalid("结果名称重复"));
        }
        let schema_ref = required_string(result, "schemaRef")?;
        let schema = schemas.resolve_for_facade(&schema_ref).map_err(|error| {
            (
                "facade_schema_invalid",
                format!("{id}.{result_id}: {error}"),
            )
        })?;
        if schema.kind != DataFrameContractKind::Result {
            return Err((
                "facade_schema_invalid",
                format!("{id}.{result_id} 的 schema 不是 result"),
            ));
        }
        result_schemas.push(schema);
        results.push(FacadeResultContract {
            id: result_id,
            name,
            schema_ref,
            description: required_string(result, "description")?,
            recommended: required_bool(result, "recommended")?,
        });
    }
    if results.is_empty() || !results.iter().any(|result| result.recommended) {
        return Err(invalid("结果 schema 和 recommended 结果不能为空"));
    }
    let constraints = required_array(value, "resultConstraints")?
        .iter()
        .map(|constraint| parse_result_constraint(constraint, &result_names))
        .collect::<Result<Vec<_>, _>>()?;
    if constraints.is_empty()
        || !constraints.iter().any(|constraint| {
            matches!(
                constraint,
                ResultSelectionRule::DefaultResults { .. }
                    | ResultSelectionRule::FixedResults { .. }
            )
        })
    {
        return Err(invalid("缺少结果默认或固定选择约束"));
    }
    let execution = json!({
        "id": id,
        "function": function,
        "family": family,
        "inputs": inputs,
        "results": results.iter().map(|result| json!({
            "id": result.id,
            "name": result.name,
            "schemaRef": result.schema_ref,
            "recommended": result.recommended,
        })).collect::<Vec<_>>(),
        "resultConstraints": constraints,
        "inputSchemas": input_schemas.iter().map(|schema| (&schema.schema_ref, &schema.schema_hash)).collect::<Vec<_>>(),
        "resultSchemas": result_schemas.iter().map(|schema| (&schema.schema_ref, &schema.schema_hash)).collect::<Vec<_>>(),
    });
    let contract_hash = hex::encode(Sha256::digest(
        serde_json::to_vec(&execution).expect("facade contract is serializable"),
    ));
    Ok(FacadeContract {
        id,
        function,
        family,
        title,
        description,
        differences,
        inputs,
        results,
        result_constraints: constraints,
        contract_hash,
    })
}

fn parse_result_constraint(
    value: &Value,
    results: &BTreeSet<String>,
) -> Result<ResultSelectionRule, (&'static str, String)> {
    let value = value
        .as_object()
        .ok_or_else(|| invalid("resultConstraints 包含非对象"))?;
    match required_string(value, "type")?.as_str() {
        "ordered_unique_selection" => Ok(ResultSelectionRule::OrderedUniqueSelection {
            selector: required_string(value, "selector")?,
            minimum: required_u32(value, "minimum")?,
            binding_count: required_string(value, "bindingCount")?,
        }),
        "default_results" => {
            let selected = required_string_array(value, "results")?;
            validate_selected_results(&selected, results)?;
            Ok(ResultSelectionRule::DefaultResults { results: selected })
        }
        "fixed_results" => {
            let selected = required_string_array(value, "results")?;
            validate_selected_results(&selected, results)?;
            let binding_count = required_u32(value, "bindingCount")?;
            if binding_count as usize != selected.len() {
                return Err(invalid("fixed_results bindingCount 与结果数量不一致"));
            }
            Ok(ResultSelectionRule::FixedResults {
                results: selected,
                binding_count,
            })
        }
        "conditional_unavailable" => {
            let result = required_string(value, "result")?;
            if !results.contains(&result) {
                return Err(invalid("conditional_unavailable 引用了未知结果"));
            }
            let when = value
                .get("when")
                .and_then(Value::as_object)
                .ok_or_else(|| invalid("conditional_unavailable.when 无效"))?;
            Ok(ResultSelectionRule::ConditionalUnavailable {
                result,
                when: ResultCondition {
                    input: required_string(when, "input")?,
                    non_empty: required_bool(when, "nonEmpty")?,
                },
            })
        }
        other => Err((
            "facade_contract_invalid",
            format!("未知 resultConstraints 类型: {other}"),
        )),
    }
}

fn validate_selected_results(
    selected: &[String],
    results: &BTreeSet<String>,
) -> Result<(), (&'static str, String)> {
    if selected.is_empty() || selected.iter().any(|result| !results.contains(result)) {
        return Err(invalid("结果选择约束为空或引用未知结果"));
    }
    Ok(())
}

fn diagnostic(
    facade_id: &str,
    code: impl Into<String>,
    message: impl Into<String>,
) -> DisabledFacadeDiagnostic {
    DisabledFacadeDiagnostic {
        facade_id: facade_id.to_string(),
        code: code.into(),
        message: message.into(),
    }
}

fn invalid(message: impl Into<String>) -> (&'static str, String) {
    ("facade_contract_invalid", message.into())
}

fn required_string(
    object: &Map<String, Value>,
    field: &str,
) -> Result<String, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("缺少或无效的 {field}")))
}

fn required_bool(object: &Map<String, Value>, field: &str) -> Result<bool, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| invalid(format!("缺少或无效的 {field}")))
}

fn required_u32(object: &Map<String, Value>, field: &str) -> Result<u32, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid(format!("缺少或无效的 {field}")))
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Vec<Value>, (&'static str, String)> {
    object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("缺少或无效的 {field}")))
}

fn required_string_array(
    object: &Map<String, Value>,
    field: &str,
) -> Result<Vec<String>, (&'static str, String)> {
    required_array(object, field)?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| invalid(format!("{field} 包含非字符串或空字符串")))
        })
        .collect()
}
