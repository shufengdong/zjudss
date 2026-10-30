use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{DataFrameContractKind, SchemaRegistry};

use super::{
    DisabledFacadeDiagnostic, FacadeInputContract, FacadeInputKind, FacadeRegistry,
    FacadeResultContract, ResultSelectionRule,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectorOutputDisposition {
    FacadeInput,
    Diagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectorOutputContract {
    pub id: String,
    pub alias: String,
    pub input_kind: FacadeInputKind,
    pub schema_ref: String,
    pub required: bool,
    pub position: u32,
    pub disposition: ProjectorOutputDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_input: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProjectionInput {
    id: String,
    alias: String,
    input_kind: String,
    schema_ref: String,
    required: bool,
    position: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseProjectionProfile {
    pub id: String,
    pub source_function: String,
    pub projector: String,
    pub target_function: String,
    pub managed_entry: String,
    pub stable_diagnostic: String,
    pub inputs: Vec<FacadeInputContract>,
    pub projector_outputs: Vec<ProjectorOutputContract>,
    pub results: Vec<FacadeResultContract>,
    pub source_results: Vec<String>,
    pub result_map: BTreeMap<String, String>,
    pub bundle_result_map: BTreeMap<String, String>,
    pub result_selector: ProjectionResultSelector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionResultSelector {
    pub input: String,
    pub field: String,
    pub value_source: String,
}

#[derive(Debug, Clone)]
pub struct CaseProjectionRegistry {
    catalog_entry_count: usize,
    enabled: BTreeMap<String, CaseProjectionProfile>,
    disabled: BTreeMap<String, DisabledFacadeDiagnostic>,
}

impl CaseProjectionRegistry {
    pub(crate) fn from_catalogs(
        root: &Value,
        facades: &FacadeRegistry,
        schemas: &mut SchemaRegistry,
    ) -> Result<Self, super::EngineResourceError> {
        let entries = root
            .get("profiles")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                super::EngineResourceError::new("catalog_invalid", "case_projections 缺少 profiles")
            })?;
        let mut enabled = BTreeMap::new();
        let mut disabled = BTreeMap::new();
        for (index, entry) in entries.iter().enumerate() {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("unknown")
                .to_string();
            let compatibility = entry.get("compatibility").and_then(Value::as_object);
            let status = compatibility
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str);
            if status == Some("disabled") {
                let code = compatibility
                    .and_then(|value| value.get("stableDiagnostic"))
                    .and_then(Value::as_str)
                    .unwrap_or("case_projection_disabled");
                disabled.insert(
                    id.clone(),
                    DisabledFacadeDiagnostic {
                        facade_id: id,
                        code: code.to_string(),
                        message: "案例投影由引擎目录禁用".to_string(),
                    },
                );
                continue;
            }
            match parse_profile(entry, facades, schemas) {
                Ok(profile) => {
                    if enabled.insert(profile.id.clone(), profile).is_some() {
                        return Err(super::EngineResourceError::new(
                            "catalog_invalid",
                            format!("case projection id 重复: {id}"),
                        ));
                    }
                }
                Err(message) => {
                    disabled.insert(
                        id.clone(),
                        DisabledFacadeDiagnostic {
                            facade_id: id,
                            code: "case_projection_invalid".to_string(),
                            message: format!("profile[{index}]: {message}"),
                        },
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

    pub fn enabled(&self) -> impl Iterator<Item = &CaseProjectionProfile> {
        self.enabled.values()
    }

    pub fn get_enabled(&self, id: &str) -> Option<&CaseProjectionProfile> {
        self.enabled.get(id)
    }

    pub fn diagnostic(&self, id: &str) -> Option<&DisabledFacadeDiagnostic> {
        self.disabled.get(id)
    }

    pub fn diagnostics(&self) -> Vec<&DisabledFacadeDiagnostic> {
        self.disabled.values().collect()
    }
}

fn parse_profile(
    value: &Value,
    facades: &FacadeRegistry,
    schemas: &mut SchemaRegistry,
) -> Result<CaseProjectionProfile, String> {
    let object = value.as_object().ok_or("profile 不是对象")?;
    let string = |name: &str| {
        object
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("缺少 {name}"))
    };
    let id = string("id")?;
    let source_function = string("sourceFunction")?;
    let projector = string("projector")?;
    let target_function = string("targetFunction")?;
    let managed_entry = string("managedEntry")?;
    if !safe_symbol(&source_function) || !safe_symbol(&projector) || !safe_symbol(&target_function)
    {
        return Err("函数或 projector 标识无效".to_string());
    }
    if managed_entry != "managed.rss" {
        return Err("managedEntry 必须是 managed.rss".to_string());
    }
    let compatibility = object
        .get("compatibility")
        .and_then(Value::as_object)
        .ok_or("缺少 compatibility")?;
    if compatibility.get("status").and_then(Value::as_str) != Some("enabled")
        || compatibility.get("failClosed").and_then(Value::as_bool) != Some(true)
    {
        return Err("enabled profile 必须 fail closed".to_string());
    }
    let stable_diagnostic = compatibility
        .get("stableDiagnostic")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("缺少 stableDiagnostic")?
        .to_string();
    let facade = facades
        .enabled()
        .find(|facade| facade.function == target_function)
        .ok_or_else(|| format!("目标门面不可用: {target_function}"))?;
    let raw_inputs: Vec<RawProjectionInput> =
        serde_json::from_value(object.get("inputs").cloned().ok_or("缺少 inputs")?)
            .map_err(|error| format!("inputs 无效: {error}"))?;
    let inputs = raw_inputs
        .into_iter()
        .map(|input| {
            Ok(FacadeInputContract {
                id: input.id,
                alias: input.alias,
                kind: parse_input_kind(&input.input_kind)?,
                schema_ref: input.schema_ref,
                required: input.required,
                position: input.position,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let results: Vec<FacadeResultContract> =
        serde_json::from_value(object.get("results").cloned().ok_or("缺少 results")?)
            .map_err(|error| format!("results 无效: {error}"))?;
    if inputs != facade.inputs || results != facade.results {
        return Err("输入或结果合同与目标门面不一致".to_string());
    }
    let mut outputs: Vec<ProjectorOutputContract> = serde_json::from_value(
        object
            .get("projectorOutputs")
            .cloned()
            .ok_or("缺少 projectorOutputs")?,
    )
    .map_err(|error| format!("projectorOutputs 无效: {error}"))?;
    if outputs.is_empty() {
        return Err("projectorOutputs 不能为空".to_string());
    }
    outputs.sort_by_key(|output| output.position);
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut mapped = BTreeSet::new();
    for (index, output) in outputs.iter().enumerate() {
        if output.position != (index + 1) as u32
            || !safe_symbol(&output.id)
            || !safe_symbol(&output.alias)
            || !ids.insert(output.id.clone())
            || !aliases.insert(output.alias.clone())
        {
            return Err("projectorOutputs 的 position、id 或 alias 无效".to_string());
        }
        let schema = schemas
            .resolve_for_facade(&output.schema_ref)
            .map_err(|error| error.to_string())?;
        let expected = match output.input_kind {
            FacadeInputKind::Model => DataFrameContractKind::Model,
            FacadeInputKind::Data => DataFrameContractKind::Data,
            FacadeInputKind::Struct => DataFrameContractKind::Struct,
        };
        if schema.kind != expected {
            return Err(format!("{} 的 inputKind 与 schema 不一致", output.id));
        }
        match output.disposition {
            ProjectorOutputDisposition::FacadeInput => {
                let target = output
                    .target_input
                    .as_deref()
                    .ok_or_else(|| format!("{} 缺少 targetInput", output.id))?;
                let input = inputs
                    .iter()
                    .find(|input| input.id == target)
                    .ok_or_else(|| format!("{} 引用未知 targetInput", output.id))?;
                if !mapped.insert(target.to_string())
                    || output.alias != input.alias
                    || output.input_kind != input.kind
                    || output.schema_ref != input.schema_ref
                    || output.required != input.required
                {
                    return Err(format!("{} 与目标门面输入不一致", output.id));
                }
            }
            ProjectorOutputDisposition::Diagnostic => {
                if output.target_input.is_some() {
                    return Err(format!("诊断输出 {} 不能声明 targetInput", output.id));
                }
            }
        }
    }
    if mapped.len() != inputs.len() {
        return Err("projectorOutputs 没有恰好覆盖所有门面输入".to_string());
    }
    let source_results: Vec<String> = serde_json::from_value(
        object
            .get("sourceResults")
            .cloned()
            .ok_or("缺少 sourceResults")?,
    )
    .map_err(|error| error.to_string())?;
    let result_map: BTreeMap<String, String> =
        serde_json::from_value(object.get("resultMap").cloned().ok_or("缺少 resultMap")?)
            .map_err(|error| error.to_string())?;
    let bundle_result_map: BTreeMap<String, String> = serde_json::from_value(
        object
            .get("bundleResultMap")
            .cloned()
            .ok_or("缺少 bundleResultMap")?,
    )
    .map_err(|error| error.to_string())?;
    if source_results.iter().collect::<BTreeSet<_>>() != result_map.keys().collect() {
        return Err("resultMap 没有覆盖 sourceResults".to_string());
    }
    let result_names = results
        .iter()
        .map(|result| &result.name)
        .collect::<BTreeSet<_>>();
    if result_map.values().any(|name| !result_names.contains(name))
        || bundle_result_map
            .values()
            .any(|name| !result_names.contains(name))
    {
        return Err("结果映射引用未知目标结果".to_string());
    }
    let result_selector: ProjectionResultSelector = serde_json::from_value(
        object
            .get("resultSelector")
            .cloned()
            .ok_or("缺少 resultSelector")?,
    )
    .map_err(|error| format!("resultSelector 无效: {error}"))?;
    let selector_input = inputs
        .iter()
        .find(|input| input.id == result_selector.input)
        .ok_or("resultSelector 引用未知输入")?;
    if selector_input.kind != FacadeInputKind::Struct
        || !safe_symbol(&result_selector.field)
        || result_selector.value_source != "profile.resultMap(manifest.study.results)"
    {
        return Err("resultSelector 合同无效".to_string());
    }
    let selector = format!("{}.{}", selector_input.alias, result_selector.field);
    if !facade.result_constraints.iter().any(|constraint| {
        matches!(
            constraint,
            ResultSelectionRule::OrderedUniqueSelection {
                selector: target,
                ..
            } if target == &selector
        )
    }) {
        return Err("resultSelector 与目标门面不一致".to_string());
    }
    Ok(CaseProjectionProfile {
        id,
        source_function,
        projector,
        target_function,
        managed_entry,
        stable_diagnostic,
        inputs,
        projector_outputs: outputs,
        results,
        source_results,
        result_map,
        bundle_result_map,
        result_selector,
    })
}

fn safe_symbol(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

fn parse_input_kind(value: &str) -> Result<FacadeInputKind, String> {
    match value {
        "model" => Ok(FacadeInputKind::Model),
        "data" => Ok(FacadeInputKind::Data),
        "struct" => Ok(FacadeInputKind::Struct),
        other => Err(format!("未知 inputKind: {other}")),
    }
}
