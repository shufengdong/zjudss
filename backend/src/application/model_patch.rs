use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::DataType;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::application::{
    ArtifactError, ArtifactMetadata, ArtifactService, DataframeBindingDraft, SourceKind,
    VersionDraft, VersionInputDraft, VersionOrigin, WorkspaceError, WorkspaceService,
};
use crate::auth::Principal;
use crate::db::models::{DataframeBinding, ModelVersion, VersionArtifact};
use crate::storage::StageLimits;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPatch {
    pub table_kind: String,
    pub row_key: BTreeMap<String, Value>,
    pub changes: BTreeMap<String, ModelChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelChange {
    pub from: Value,
    pub to: Value,
    pub unit: String,
}

#[derive(Debug)]
pub struct ModelMutationError {
    code: &'static str,
    message: String,
}

impl ModelMutationError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for ModelMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ModelMutationError {}

impl From<WorkspaceError> for ModelMutationError {
    fn from(value: WorkspaceError) -> Self {
        let code = if matches!(value, WorkspaceError::VersionConflict) {
            "version_conflict"
        } else {
            "workspace_error"
        };
        Self::new(code, value.to_string())
    }
}
impl From<ArtifactError> for ModelMutationError {
    fn from(value: ArtifactError) -> Self {
        Self::new("artifact_error", value.to_string())
    }
}

#[derive(Clone)]
pub struct ModelPatchService {
    workspaces: WorkspaceService,
    artifacts: ArtifactService,
}

impl ModelPatchService {
    pub fn new(workspaces: WorkspaceService, artifacts: ArtifactService) -> Self {
        Self {
            workspaces,
            artifacts,
        }
    }

    pub async fn confirm_mapping(
        &self,
        principal: &Principal,
        session_id: i32,
        input_id: i32,
        table_kind: String,
        expected_base: Option<i32>,
    ) -> Result<ModelVersion, ModelMutationError> {
        let (record, base, mut inputs, manifest) =
            self.base(principal, session_id, expected_base).await?;
        let target = inputs
            .iter_mut()
            .find(|input| input.input.id == input_id)
            .ok_or_else(|| ModelMutationError::new("input_not_found", "inputId 不属于当前基线"))?;
        let binding = target.binding.as_mut().ok_or_else(|| {
            ModelMutationError::new("mapping_not_applicable", "该输入不是 DataFrame")
        })?;
        let evidence: Value = serde_json::from_str(&binding.evidence_json).unwrap_or_default();
        let is_candidate = evidence
            .get("candidates")
            .and_then(Value::as_array)
            .is_some_and(|candidates| {
                candidates.iter().any(|candidate| {
                    candidate.get("tableKind").and_then(Value::as_str) == Some(table_kind.as_str())
                })
            });
        if !is_candidate && binding.table_kind.as_deref() != Some(table_kind.as_str()) {
            return Err(ModelMutationError::new(
                "mapping_invalid",
                "tableKind 不在当前推断候选中",
            ));
        }
        binding.table_kind = Some(table_kind);
        binding.resolution = "user_confirmed".into();
        self.publish(
            principal,
            record.workspace.id,
            Some(base.id),
            &base,
            manifest,
            inputs,
            None,
        )
        .await
    }

    pub async fn remove_input(
        &self,
        principal: &Principal,
        session_id: i32,
        input_id: i32,
        expected_base: Option<i32>,
    ) -> Result<ModelVersion, ModelMutationError> {
        let (record, base, mut inputs, manifest) =
            self.base(principal, session_id, expected_base).await?;
        let before = inputs.len();
        inputs.retain(|input| input.input.id != input_id);
        if inputs.len() == before {
            return Err(ModelMutationError::new(
                "input_not_found",
                "inputId 不属于当前基线",
            ));
        }
        if inputs.is_empty() {
            return Err(ModelMutationError::new(
                "model_empty",
                "不能移除最后一个模型输入",
            ));
        }
        self.publish(
            principal,
            record.workspace.id,
            Some(base.id),
            &base,
            manifest,
            inputs,
            None,
        )
        .await
    }

    pub async fn replace_input(
        &self,
        principal: &Principal,
        session_id: i32,
        target_input_id: i32,
        replacement_input_id: i32,
        expected_base: Option<i32>,
    ) -> Result<ModelVersion, ModelMutationError> {
        if target_input_id == replacement_input_id {
            return Err(ModelMutationError::new(
                "replacement_invalid",
                "目标与替换 inputId 不能相同",
            ));
        }
        let (record, base, mut inputs, manifest) =
            self.base(principal, session_id, expected_base).await?;
        let replacement = inputs
            .iter()
            .find(|input| input.input.id == replacement_input_id)
            .cloned()
            .ok_or_else(|| {
                ModelMutationError::new("input_not_found", "replacementInputId 不属于当前基线")
            })?;
        let target_index = inputs
            .iter()
            .position(|input| input.input.id == target_input_id)
            .ok_or_else(|| {
                ModelMutationError::new("input_not_found", "targetInputId 不属于当前基线")
            })?;
        inputs[target_index] = replacement;
        inputs.remove(
            inputs
                .iter()
                .position(|input| input.input.id == replacement_input_id)
                .expect("replacement remains"),
        );
        self.publish(
            principal,
            record.workspace.id,
            Some(base.id),
            &base,
            manifest,
            inputs,
            None,
        )
        .await
    }

    pub async fn modify(
        &self,
        principal: &Principal,
        session_id: i32,
        input_id: i32,
        patch: ModelPatch,
        expected_base: Option<i32>,
    ) -> Result<ModelVersion, ModelMutationError> {
        if patch.row_key.is_empty() || patch.changes.is_empty() {
            return Err(ModelMutationError::new(
                "invalid_model_patch",
                "rowKey 和 changes 不能为空",
            ));
        }
        let (record, base, mut inputs, manifest) =
            self.base(principal, session_id, expected_base).await?;
        let target_index = inputs
            .iter()
            .position(|input| input.input.id == input_id)
            .ok_or_else(|| ModelMutationError::new("input_not_found", "inputId 不属于当前基线"))?;
        let target = &inputs[target_index];
        validate_units(&patch)?;
        let scope = self.artifacts.resolve_scope(&principal.storage).await?;
        let artifact = self
            .artifacts
            .find_owned(scope.id, target.input.artifact_id)
            .await?;
        let source = self
            .artifacts
            .store()
            .resolve(&artifact.storage_path)
            .map_err(|e| ModelMutationError::new("artifact_error", e.to_string()))?;
        let (bytes, output_format, media_type) = match artifact.format.as_str() {
            "csv" | "parquet" => {
                let binding = target.binding.as_ref().ok_or_else(|| {
                    ModelMutationError::new("invalid_model_patch", "该 DataFrame 输入没有类型映射")
                })?;
                if binding.table_kind.as_deref() != Some(patch.table_kind.as_str()) {
                    return Err(ModelMutationError::new(
                        "invalid_model_patch",
                        "tableKind 与已确认映射不一致",
                    ));
                }
                let bytes = if artifact.format == "csv" {
                    patch_csv(&source, &patch)?
                } else {
                    patch_parquet(&source, &patch)?
                };
                (bytes, "parquet", Some("application/vnd.apache.parquet"))
            }
            "rss" if patch.table_kind == "rss_parameter" => (
                patch_assignment_text(&source, &patch, "rss_parameter")?,
                "rss",
                Some("text/plain"),
            ),
            "dss" if patch.table_kind == "dss_property" => (
                patch_assignment_text(&source, &patch, "dss_property")?,
                "dss",
                Some("text/plain"),
            ),
            _ => {
                return Err(ModelMutationError::new(
                    "invalid_model_patch",
                    "当前输入类型不支持确定性结构化修改",
                ));
            }
        };
        let staged = self
            .artifacts
            .store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .map_err(|e| ModelMutationError::new("artifact_error", e.to_string()))?;
        let committed = self
            .artifacts
            .commit(
                scope.id,
                staged,
                ArtifactMetadata {
                    format: output_format.into(),
                    media_type: media_type.map(str::to_string),
                },
            )
            .await?;
        inputs[target_index].input.artifact_id = committed.id;
        inputs[target_index].sha256 = committed.sha256.clone();
        inputs[target_index].input.display_name =
            format!("patched-input-{input_id}.{output_format}");
        if output_format == "parquet" {
            inputs[target_index].input.relative_path =
                Some(format!("generated/input-{input_id}.parquet"));
        }
        let patch_value = serde_json::to_value(&patch).expect("patch serializable");
        match self
            .publish(
                principal,
                record.workspace.id,
                Some(base.id),
                &base,
                manifest,
                inputs,
                Some(patch_value),
            )
            .await
        {
            Ok(version) => Ok(version),
            Err(error) => {
                let _ = self.artifacts.release(scope.id, committed.id).await;
                Err(error)
            }
        }
    }

    async fn base(
        &self,
        principal: &Principal,
        session_id: i32,
        expected_base: Option<i32>,
    ) -> Result<
        (
            crate::db::repositories::WorkspaceRecord,
            ModelVersion,
            Vec<InputState>,
            Value,
        ),
        ModelMutationError,
    > {
        let record = self.workspaces.get(&principal.access, session_id).await?;
        let current = record
            .workspace
            .draft_version_id
            .or(record.workspace.active_version_id);
        if current.is_some() && expected_base.is_none() {
            return Err(ModelMutationError::new(
                "expected_version_required",
                "expectedDraftVersionId 必填",
            ));
        }
        if current != expected_base {
            return Err(ModelMutationError::new(
                "version_conflict",
                "模型版本基线已变化",
            ));
        }
        let base_id = current
            .ok_or_else(|| ModelMutationError::new("workspace_empty", "工作区尚无模型版本"))?;
        let base = self
            .workspaces
            .find_version(&principal.access, base_id)
            .await?;
        let manifest: Value = serde_json::from_str(&base.manifest_json)
            .map_err(|_| ModelMutationError::new("invalid_manifest", "版本 manifest 无效"))?;
        let mut states = Vec::new();
        let scope = self.artifacts.resolve_scope(&principal.storage).await?;
        for (input, binding) in self.workspaces.inputs(&principal.access, base.id).await? {
            let artifact = self
                .artifacts
                .find_owned(scope.id, input.artifact_id)
                .await?;
            let source_sha = match input.source_artifact_id {
                Some(id) => Some(self.artifacts.find_owned(scope.id, id).await?.sha256),
                None => None,
            };
            states.push(InputState {
                input,
                binding,
                sha256: artifact.sha256,
                source_sha256: source_sha,
            });
        }
        Ok((record, base, states, clean_manifest(manifest)))
    }

    async fn publish(
        &self,
        principal: &Principal,
        workspace_id: i32,
        expected: Option<i32>,
        base: &ModelVersion,
        manifest: Value,
        inputs: Vec<InputState>,
        patch: Option<Value>,
    ) -> Result<ModelVersion, ModelMutationError> {
        let status = if base.status == "ready" {
            crate::application::ModelVersionStatus::Ready
        } else {
            crate::application::ModelVersionStatus::Collecting
        };
        self.workspaces
            .publish_version(
                &principal.access,
                workspace_id,
                expected,
                VersionDraft {
                    status,
                    source_kind: source_kind(&base.source_kind)?,
                    origin: VersionOrigin::Generated,
                    manifest,
                    patch,
                    inputs: inputs.into_iter().map(InputState::draft).collect(),
                },
            )
            .await
            .map_err(Into::into)
    }
}

#[derive(Clone)]
struct InputState {
    input: VersionArtifact,
    binding: Option<DataframeBinding>,
    sha256: String,
    source_sha256: Option<String>,
}

impl InputState {
    fn draft(self) -> VersionInputDraft {
        VersionInputDraft {
            artifact_id: self.input.artifact_id,
            sha256: self.sha256,
            role: self.input.role,
            relative_path: self.input.relative_path,
            display_name: self.input.display_name,
            source_artifact_id: self.input.source_artifact_id,
            source_artifact_sha256: self.source_sha256,
            sheet_name: self.input.sheet_name,
            binding: self.binding.map(|binding| DataframeBindingDraft {
                table_kind: binding.table_kind,
                confidence: binding.confidence,
                resolution: binding.resolution,
                columns: serde_json::from_str(&binding.columns_json).unwrap_or_else(|_| json!([])),
                evidence: serde_json::from_str(&binding.evidence_json)
                    .unwrap_or_else(|_| json!({})),
            }),
        }
    }
}

fn clean_manifest(mut manifest: Value) -> Value {
    if let Some(object) = manifest.as_object_mut() {
        for key in ["inputs", "origin", "originKind", "patch"] {
            object.remove(key);
        }
    }
    manifest
}

fn source_kind(value: &str) -> Result<SourceKind, ModelMutationError> {
    match value {
        "rss" => Ok(SourceKind::Rss),
        "rssbundle" => Ok(SourceKind::Rssbundle),
        "dataframes" => Ok(SourceKind::Dataframes),
        "mixed" => Ok(SourceKind::Mixed),
        _ => Err(ModelMutationError::new(
            "invalid_manifest",
            "未知 sourceKind",
        )),
    }
}

fn validate_units(patch: &ModelPatch) -> Result<(), ModelMutationError> {
    for (column, change) in &patch.changes {
        let expected = expected_dimension(column);
        let actual = unit_dimension(&change.unit);
        let text_patch = matches!(patch.table_kind.as_str(), "rss_parameter" | "dss_property");
        if actual.is_none()
            || (!text_patch && expected.is_none())
            || expected.is_some_and(|value| Some(value) != actual)
        {
            return Err(ModelMutationError::new(
                "unit_incompatible",
                format!("{column} 的单位 {} 不兼容", change.unit),
            ));
        }
        if change.from.is_number() != change.to.is_number()
            || change.from.is_string() != change.to.is_string()
            || change.from.is_boolean() != change.to.is_boolean()
        {
            return Err(ModelMutationError::new(
                "invalid_model_patch",
                format!("{column} 的 from/to 类型不一致"),
            ));
        }
    }
    Ok(())
}

fn expected_dimension(column: &str) -> Option<&'static str> {
    let name = column.to_ascii_lowercase();
    if matches!(name.as_str(), "p" | "p_kw" | "kw" | "active_power") {
        Some("power")
    } else if matches!(name.as_str(), "q" | "q_kvar" | "kvar" | "reactive_power") {
        Some("reactive_power")
    } else if name.contains("voltage") || matches!(name.as_str(), "v" | "v_mag") {
        Some("voltage")
    } else if name.contains("current") || matches!(name.as_str(), "i" | "i_mag") {
        Some("current")
    } else if matches!(name.as_str(), "r" | "x" | "resistance" | "reactance") {
        Some("resistance")
    } else if name.contains("angle") {
        Some("angle")
    } else if name.contains("time") || name.contains("step") || name.contains("duration") {
        Some("time")
    } else {
        None
    }
}

fn unit_dimension(unit: &str) -> Option<&'static str> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "w" | "kw" | "mw" => Some("power"),
        "var" | "kvar" | "mvar" => Some("reactive_power"),
        "v" | "kv" => Some("voltage"),
        "a" | "ka" => Some("current"),
        "ohm" | "ω" => Some("resistance"),
        "rad" | "degree" | "deg" => Some("angle"),
        "s" | "min" | "h" => Some("time"),
        "1" | "pu" | "%" => Some("dimensionless"),
        _ => None,
    }
}

fn patch_csv(path: &std::path::Path, patch: &ModelPatch) -> Result<Vec<u8>, ModelMutationError> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let headers = reader
        .headers()
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?
        .clone();
    let mut rows: Vec<Vec<String>> = reader
        .records()
        .map(|row| row.map(|r| r.iter().map(str::to_string).collect()))
        .collect::<Result<_, _>>()
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let matches: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row_matches(&headers, row, &patch.row_key))
        .map(|(i, _)| i)
        .collect();
    if matches.len() != 1 {
        return Err(ModelMutationError::new(
            "row_not_unique",
            format!("rowKey 匹配 {} 行", matches.len()),
        ));
    }
    let row = &mut rows[matches[0]];
    for (column, change) in &patch.changes {
        let index = headers
            .iter()
            .position(|name| name == column)
            .ok_or_else(|| {
                ModelMutationError::new("invalid_model_patch", format!("列不存在: {column}"))
            })?;
        if !text_equals(&row[index], &change.from) {
            return Err(ModelMutationError::new(
                "old_value_mismatch",
                format!("{column} 的旧值不符"),
            ));
        }
        row[index] = scalar_text(&change.to)?;
    }
    string_rows_to_parquet(headers.iter().map(str::to_string).collect(), rows)
}

fn row_matches(headers: &csv::StringRecord, row: &[String], key: &BTreeMap<String, Value>) -> bool {
    key.iter().all(|(name, value)| {
        headers
            .iter()
            .position(|header| header == name)
            .is_some_and(|i| text_equals(&row[i], value))
    })
}

fn text_equals(actual: &str, expected: &Value) -> bool {
    match expected {
        Value::String(value) => actual == value,
        Value::Number(value) => actual.parse::<f64>().ok() == value.as_f64(),
        Value::Bool(value) => actual.parse::<bool>().ok() == Some(*value),
        _ => false,
    }
}
fn scalar_text(value: &Value) -> Result<String, ModelMutationError> {
    match value {
        Value::String(v) => Ok(v.clone()),
        Value::Number(v) => Ok(v.to_string()),
        Value::Bool(v) => Ok(v.to_string()),
        _ => Err(ModelMutationError::new(
            "invalid_model_patch",
            "只支持标量值",
        )),
    }
}

fn patch_assignment_text(
    path: &std::path::Path,
    patch: &ModelPatch,
    kind: &str,
) -> Result<Vec<u8>, ModelMutationError> {
    if patch.row_key.len() != 1 || patch.changes.len() != 1 {
        return Err(ModelMutationError::new(
            "invalid_model_patch",
            "文本补丁必须唯一指定对象和属性",
        ));
    }
    let selector = if kind == "rss_parameter" {
        "name"
    } else {
        "object"
    };
    let target = patch
        .row_key
        .get(selector)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ModelMutationError::new(
                "invalid_model_patch",
                format!("{kind} 需要字符串 {selector}"),
            )
        })?;
    let (property, change) = patch.changes.iter().next().unwrap();
    if kind == "rss_parameter" && property != "value" {
        return Err(ModelMutationError::new(
            "invalid_model_patch",
            "RSS 参数补丁只允许 changes.value",
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let mut matches = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        let matched = if kind == "rss_parameter" {
            trimmed
                .split_once('=')
                .is_some_and(|(name, _)| name.trim() == target)
        } else {
            let mut words = trimmed.split_whitespace();
            words.next().is_some_and(|word| {
                word.eq_ignore_ascii_case("new") || word.eq_ignore_ascii_case("edit")
            }) && words
                .next()
                .is_some_and(|object| object.eq_ignore_ascii_case(target))
                && trimmed.split_whitespace().any(|word| {
                    word.split_once('=')
                        .is_some_and(|(name, _)| name.eq_ignore_ascii_case(property))
                })
        };
        if matched {
            matches.push(index);
        }
    }
    if matches.len() != 1 {
        return Err(ModelMutationError::new(
            "row_not_unique",
            format!("文本选择器匹配 {} 行", matches.len()),
        ));
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let line = &lines[matches[0]];
    if kind == "rss_parameter" {
        let (left, right) = line.split_once('=').unwrap();
        let current = right.trim().trim_end_matches(';').trim();
        if !text_equals(current.trim_matches('"'), &change.from) {
            return Err(ModelMutationError::new(
                "old_value_mismatch",
                format!("{target} 的旧值不符"),
            ));
        }
        lines[matches[0]] = format!("{}= {};", left, scalar_text(&change.to)?);
    } else {
        let mut changed = false;
        let mut words = Vec::new();
        for word in line.split_whitespace() {
            if let Some((name, value)) = word.split_once('=') {
                if name.eq_ignore_ascii_case(property) {
                    if !text_equals(value.trim_end_matches(';').trim_matches('"'), &change.from) {
                        return Err(ModelMutationError::new(
                            "old_value_mismatch",
                            format!("{property} 的旧值不符"),
                        ));
                    }
                    words.push(format!("{name}={}", scalar_text(&change.to)?));
                    changed = true;
                    continue;
                }
            }
            words.push(word.to_string());
        }
        if !changed {
            return Err(ModelMutationError::new("invalid_model_patch", "属性不存在"));
        }
        lines[matches[0]] = words.join(" ");
    }
    let mut result = lines.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    Ok(result.into_bytes())
}

fn string_rows_to_parquet(
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
) -> Result<Vec<u8>, ModelMutationError> {
    let schema = Arc::new(arrow_schema::Schema::new(
        headers
            .iter()
            .map(|name| arrow_schema::Field::new(name, DataType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let arrays: Vec<ArrayRef> = (0..headers.len())
        .map(|column| {
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|row| row[column].as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef
        })
        .collect();
    let batch = RecordBatch::try_new(schema.clone(), arrays)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let mut output = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut output, schema, None)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    Ok(output)
}

fn patch_parquet(
    path: &std::path::Path,
    patch: &ModelPatch,
) -> Result<Vec<u8>, ModelMutationError> {
    let file = std::fs::File::open(path)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let schema = builder.schema().clone();
    let batches: Vec<RecordBatch> = builder
        .build()
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    let mut found = Vec::new();
    for (bi, batch) in batches.iter().enumerate() {
        for row in 0..batch.num_rows() {
            if patch.row_key.iter().all(|(name, expected)| {
                schema
                    .index_of(name)
                    .ok()
                    .and_then(|column| scalar_at(batch.column(column), row))
                    .is_some_and(|actual| actual == *expected)
            }) {
                found.push((bi, row));
            }
        }
    }
    if found.len() != 1 {
        return Err(ModelMutationError::new(
            "row_not_unique",
            format!("rowKey 匹配 {} 行", found.len()),
        ));
    }
    let (target_batch, target_row) = found[0];
    let mut output_batches = Vec::new();
    for (bi, batch) in batches.into_iter().enumerate() {
        if bi != target_batch {
            output_batches.push(batch);
            continue;
        }
        let mut columns = batch.columns().to_vec();
        for (name, change) in &patch.changes {
            let index = schema.index_of(name).map_err(|_| {
                ModelMutationError::new("invalid_model_patch", format!("列不存在: {name}"))
            })?;
            let actual = scalar_at(&columns[index], target_row).ok_or_else(|| {
                ModelMutationError::new("invalid_model_patch", "不支持的列类型或 null")
            })?;
            if actual != change.from {
                return Err(ModelMutationError::new(
                    "old_value_mismatch",
                    format!("{name} 的旧值不符"),
                ));
            }
            columns[index] = replace_scalar(&columns[index], target_row, &change.to)?;
        }
        output_batches.push(
            RecordBatch::try_new(schema.clone(), columns)
                .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?,
        );
    }
    let mut output = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut output, schema, None)
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    for batch in output_batches {
        writer
            .write(&batch)
            .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    }
    writer
        .close()
        .map_err(|e| ModelMutationError::new("invalid_model_patch", e.to_string()))?;
    Ok(output)
}

fn scalar_at(array: &ArrayRef, row: usize) -> Option<Value> {
    if array.is_null(row) {
        return None;
    }
    match array.data_type() {
        DataType::Utf8 => Some(json!(
            array.as_any().downcast_ref::<StringArray>()?.value(row)
        )),
        DataType::Int64 => Some(json!(
            array.as_any().downcast_ref::<Int64Array>()?.value(row)
        )),
        DataType::Float64 => {
            serde_json::Number::from_f64(array.as_any().downcast_ref::<Float64Array>()?.value(row))
                .map(Value::Number)
        }
        DataType::Boolean => Some(json!(
            array.as_any().downcast_ref::<BooleanArray>()?.value(row)
        )),
        _ => None,
    }
}

fn replace_scalar(
    array: &ArrayRef,
    row: usize,
    value: &Value,
) -> Result<ArrayRef, ModelMutationError> {
    let invalid = || ModelMutationError::new("invalid_model_patch", "to 值与列类型不兼容");
    match array.data_type() {
        DataType::Utf8 => {
            let old = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(invalid)?;
            let new = scalar_text(value)?;
            Ok(Arc::new(StringArray::from(
                (0..old.len())
                    .map(|i| {
                        if i == row {
                            Some(new.as_str())
                        } else if old.is_null(i) {
                            None
                        } else {
                            Some(old.value(i))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        DataType::Int64 => {
            let old = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(invalid)?;
            let new = value.as_i64().ok_or_else(invalid)?;
            Ok(Arc::new(Int64Array::from(
                (0..old.len())
                    .map(|i| {
                        if i == row {
                            Some(new)
                        } else if old.is_null(i) {
                            None
                        } else {
                            Some(old.value(i))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        DataType::Float64 => {
            let old = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(invalid)?;
            let new = value.as_f64().ok_or_else(invalid)?;
            Ok(Arc::new(Float64Array::from(
                (0..old.len())
                    .map(|i| {
                        if i == row {
                            Some(new)
                        } else if old.is_null(i) {
                            None
                        } else {
                            Some(old.value(i))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        DataType::Boolean => {
            let old = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(invalid)?;
            let new = value.as_bool().ok_or_else(invalid)?;
            Ok(Arc::new(BooleanArray::from(
                (0..old.len())
                    .map(|i| {
                        if i == row {
                            Some(new)
                        } else if old.is_null(i) {
                            None
                        } else {
                            Some(old.value(i))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        _ => Err(invalid()),
    }
}
