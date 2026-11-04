use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::adapters::{
    CaseProjectionSpec, ImportSpec, MODEL_TABLE_OUTPUTS, PARA_RESOURCE_OUTPUT,
    ProjectionOutputSpec, SimEngine,
};
use crate::auth::CurrentUser;
use crate::catalog::DataFrameContractKind;
use crate::db::repositories::{NewUploadCandidate, UploadBatchRecordSet, UploadRepo};
use crate::engine_resources::{
    EngineExample, EngineResourceSnapshot, FacadeInputKind, ProjectorOutputDisposition,
};
use crate::ingestion::{
    ArchiveExtractor, ArchiveLimits, normalize_csv_with_settings, normalize_excel_to_parquet,
};
use crate::storage::{ParquetInspector, StageLimits};

use super::resource_alias::normalize_alias;
use super::uploads::{
    CandidatePurpose, ParseSettings, UploadBatchError, source_fingerprint, stable_id,
};
use super::{ArtifactMetadata, ArtifactService, now_ms};

struct ParsedTabularCandidate {
    file_id: String,
    sheet: Option<String>,
    display: String,
    path: PathBuf,
    metadata: crate::storage::ParquetMetadata,
    alias: Option<String>,
    purpose: Option<CandidatePurpose>,
    schema_ref: Option<String>,
}

#[derive(Clone)]
pub(super) struct UploadProcessor {
    repo: UploadRepo,
    artifacts: ArtifactService,
    engine: Arc<dyn SimEngine>,
    snapshot: Arc<EngineResourceSnapshot>,
    data_dir: PathBuf,
    archive_limits: ArchiveLimits,
}

impl UploadProcessor {
    pub(super) fn new(
        repo: UploadRepo,
        artifacts: ArtifactService,
        engine: Arc<dyn SimEngine>,
        snapshot: Arc<EngineResourceSnapshot>,
        data_dir: PathBuf,
        archive_limits: ArchiveLimits,
    ) -> Self {
        Self {
            repo,
            artifacts,
            engine,
            snapshot,
            data_dir,
            archive_limits,
        }
    }

    pub(super) fn artifact_store(&self) -> &crate::storage::ArtifactStore {
        self.artifacts.store()
    }

    pub(super) async fn process<F>(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
        case_switch_state: F,
    ) -> Result<(), UploadBatchError>
    where
        F: Future<Output = Result<serde_json::Value, UploadBatchError>>,
    {
        self.validate_settings(settings)?;
        let all_dss = record
            .files
            .iter()
            .all(|file| extension(&file.file_name) == "dss");
        let directory_bundle = record.files.iter().any(|file| {
            file.relative_path.as_deref().is_some_and(|path| {
                Path::new(path).components().any(|component| {
                    let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
                    value.ends_with(".dssbundle") || value.ends_with(".rssbundle")
                })
            })
        });
        if record.batch.source_kind == "case" {
            self.process_case(user, record, settings, case_switch_state)
                .await
        } else if record.files.len() == 1 && extension(&record.files[0].file_name) == "zip" {
            self.process_archive(user, record, settings).await
        } else if all_dss || directory_bundle {
            self.process_dss(user, record, settings).await
        } else {
            self.process_tabular(user, record, settings).await
        }
    }

    pub(super) fn validate_settings(
        &self,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        if !settings.encoding.eq_ignore_ascii_case("utf-8")
            || !matches!(settings.decimal.as_str(), "." | ",")
            || settings
                .delimiter
                .is_some_and(|value| !matches!(value, b',' | b';' | b'\t' | b'|'))
        {
            return Err(UploadBatchError::Invalid(
                "upload_parse_settings_invalid".into(),
                "解析设置不受支持".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn diagnostics(
        &self,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
        error: &UploadBatchError,
    ) -> serde_json::Value {
        batch_diagnostics(record, settings, Some(error))
    }

    async fn process_dss(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let input = self.materialize_tree(record)?;
        let file_id = record
            .files
            .iter()
            .find(|file| extension(&file.file_name) == "dss")
            .map(|file| file.id.as_str())
            .unwrap_or(&record.files[0].id);
        self.process_dss_tree(user, record, file_id, input, settings)
            .await
    }

    async fn process_case<F>(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
        case_switch_state: F,
    ) -> Result<(), UploadBatchError>
    where
        F: Future<Output = Result<serde_json::Value, UploadBatchError>>,
    {
        let example = self.case_example(record)?.clone();
        let manifest = example.manifest.as_ref().ok_or_else(|| {
            UploadBatchError::Invalid(
                "case_projection_invalid".into(),
                "案例缺少已验证的 projection manifest".into(),
            )
        })?;
        let profile = self
            .snapshot
            .case_projections
            .get_enabled(&manifest.projection.profile)
            .ok_or_else(|| {
                UploadBatchError::Invalid(
                    "case_projection_unavailable".into(),
                    "案例引用的投影配置在当前快照中不可用".into(),
                )
            })?
            .clone();
        let input = self.materialize_case_tree(record)?;
        let source_file = record
            .files
            .iter()
            .find(|file| file.relative_path.as_deref() == Some(manifest.source.entry.as_str()))
            .ok_or_else(|| {
                UploadBatchError::Invalid(
                    "case_projection_source_missing".into(),
                    "案例投影源入口未进入批次".into(),
                )
            })?;
        let results = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join("results");
        let projected = self
            .engine
            .project_case(CaseProjectionSpec {
                input_tree: input,
                source_entry: PathBuf::from(&manifest.source.entry),
                projector: profile.projector.clone(),
                outputs: profile
                    .projector_outputs
                    .iter()
                    .map(|output| ProjectionOutputSpec {
                        alias: output.alias.clone(),
                        diagnostic: output.disposition == ProjectorOutputDisposition::Diagnostic,
                    })
                    .collect(),
                result_dir: results,
                timeout: std::time::Duration::ZERO,
            })
            .await
            .map_err(|error| UploadBatchError::Invalid(error.code.clone(), error.to_string()))?;
        let projection_diagnostics = read_projection_diagnostics(&projected.diagnostics)?;
        let mut candidates = Vec::new();
        let mut committed = Vec::new();
        for dataset in projected.dataframes {
            let output = profile
                .projector_outputs
                .iter()
                .find(|output| output.alias == dataset.name)
                .ok_or_else(|| {
                    UploadBatchError::Invalid(
                        "case_projection_output_mismatch".into(),
                        "案例投影输出未出现在已发布合同中".into(),
                    )
                })?;
            let purpose = match output.input_kind {
                FacadeInputKind::Model => CandidatePurpose::ModelTable,
                FacadeInputKind::Data | FacadeInputKind::Struct => CandidatePurpose::DataResource,
            };
            let artifact = match self.commit_path(user.storage_scope_id, &dataset.path).await {
                Ok(value) => value,
                Err(error) => {
                    self.release_ids(user.storage_scope_id, &committed).await;
                    return Err(error);
                }
            };
            committed.push(artifact.id);
            candidates.push(candidate(
                record.batch.id,
                &source_file.id,
                None,
                &dataset.name,
                Some(&output.alias),
                Some(purpose),
                Some(output.schema_ref.clone()),
                artifact.id,
                &dataset.metadata,
                serde_json::json!({
                    "source": "case_projection",
                    "alias": output.alias,
                    "exampleId": example.id,
                    "profile": profile.id,
                    "projector": profile.projector,
                    "targetFunction": profile.target_function,
                    "position": output.position,
                    "inputKind": output.input_kind,
                    "disposition": output.disposition,
                    "engineResourceSnapshot": self.snapshot.id
                }),
            ));
        }
        let mut diagnostics = batch_diagnostics(record, settings, None);
        diagnostics["caseSwitch"] = case_switch_state.await?;
        diagnostics["caseProjection"] = serde_json::json!({
            "exampleId": example.id,
            "title": example.title,
            "profile": profile.id,
            "projector": profile.projector,
            "targetFunction": profile.target_function,
            "diagnosticOutputs": projection_diagnostics
        });
        if let Err(error) = self
            .repo
            .replace_candidates(
                record.batch.id,
                candidates,
                "ready_to_confirm",
                serde_json::to_string(settings).unwrap(),
                diagnostics.to_string(),
                now_ms(),
            )
            .await
        {
            self.release_ids(user.storage_scope_id, &committed).await;
            return Err(error.into());
        }
        self.release_previous(user.storage_scope_id, record).await;
        Ok(())
    }

    fn case_example<'a>(
        &'a self,
        record: &UploadBatchRecordSet,
    ) -> Result<&'a EngineExample, UploadBatchError> {
        self.snapshot
            .examples
            .iter()
            .find(|example| {
                example.importable
                    && example.files.len() == record.files.len()
                    && example.files.iter().all(|source| {
                        record.files.iter().any(|file| {
                            file.relative_path.as_deref() == Some(source.virtual_path.as_str())
                                && file.sha256 == source.sha256
                        })
                    })
            })
            .ok_or_else(|| {
                UploadBatchError::Invalid(
                    "case_snapshot_contract_mismatch".into(),
                    "案例批次与已发布引擎资源快照不一致".into(),
                )
            })
    }

    async fn process_archive(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let file = &record.files[0];
        let source = self.data_dir.join(&file.staged_path);
        let destination = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join("archive");
        let tree = ArchiveExtractor::extract(&source, &destination, self.archive_limits).map_err(
            |error| UploadBatchError::Invalid("archive_limits_exceeded".into(), error.to_string()),
        )?;
        let entries = tree.files.values().collect::<Vec<_>>();
        let all_dss = !entries.is_empty()
            && entries
                .iter()
                .all(|entry| extension(&entry.relative_path) == "dss");
        let recognized_bundle = entries
            .iter()
            .any(|entry| extension(&entry.relative_path) == "dss")
            && entries.iter().any(|entry| {
                std::path::Path::new(&entry.relative_path)
                    .components()
                    .any(|component| {
                        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
                        value.ends_with(".dssbundle") || value.ends_with(".rssbundle")
                    })
            })
            && entries.iter().all(|entry| {
                matches!(
                    extension(&entry.relative_path).as_str(),
                    "dss" | "rss" | "json"
                )
            });
        if !all_dss && !recognized_bundle {
            return Err(UploadBatchError::Invalid(
                "upload_format_unsupported".into(),
                "归档必须包含一个 DSS 文件树".into(),
            ));
        }
        self.process_dss_tree(user, record, &file.id, tree.root, settings)
            .await
    }

    async fn process_dss_tree(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        file_id: &str,
        input: PathBuf,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let inspection = self
            .engine
            .inspect_model_import(crate::adapters::ImportInspectSpec {
                input_tree: input.clone(),
                timeout: std::time::Duration::ZERO,
            })
            .await
            .map_err(|e| UploadBatchError::Invalid(e.code.clone(), e.to_string()))?;
        if inspection.entry_candidates.len() != 1 {
            return Err(UploadBatchError::Invalid(
                "model_import_resolution_required".into(),
                "DSS 入口不唯一".into(),
            ));
        }
        let results = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join("results");
        let imported = self
            .engine
            .import_model(ImportSpec {
                input_tree: input,
                entry_candidate_id: inspection.entry_candidates[0].id.clone(),
                result_dir: results,
                timeout: std::time::Duration::ZERO,
            })
            .await
            .map_err(|e| UploadBatchError::Invalid(e.code.clone(), e.to_string()))?;
        let mut candidates = Vec::new();
        let mut committed = Vec::new();
        for output in imported.dataframes {
            let is_para = output.name == PARA_RESOURCE_OUTPUT;
            if !is_para && !MODEL_TABLE_OUTPUTS.contains(&output.name.as_str()) {
                continue;
            }
            let artifact = match self.commit_path(user.storage_scope_id, &output.path).await {
                Ok(value) => value,
                Err(error) => {
                    self.release_ids(user.storage_scope_id, &committed).await;
                    return Err(error);
                }
            };
            committed.push(artifact.id);
            candidates.push(candidate(
                record.batch.id,
                file_id,
                None,
                &output.name,
                Some(&output.name),
                Some(if is_para {
                    CandidatePurpose::DataResource
                } else {
                    CandidatePurpose::ModelTable
                }),
                Some(format!("frames.{}", output.name)),
                artifact.id,
                &output.metadata,
                serde_json::json!({"source":"static_import","profile":imported.profile}),
            ));
        }
        if let Err(error) = self
            .repo
            .replace_candidates(
                record.batch.id,
                candidates,
                "ready_to_confirm",
                serde_json::to_string(settings).unwrap(),
                batch_diagnostics(record, settings, None).to_string(),
                now_ms(),
            )
            .await
        {
            self.release_ids(user.storage_scope_id, &committed).await;
            return Err(error.into());
        }
        self.release_previous(user.storage_scope_id, record).await;
        Ok(())
    }

    async fn process_tabular(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let mut parsed = Vec::new();
        let mut needs_settings = false;
        let mut needs_purpose = false;
        if record.files.iter().any(|file| {
            !matches!(
                extension(&file.file_name).as_str(),
                "csv" | "xls" | "xlsx" | "parquet"
            )
        }) {
            return Err(UploadBatchError::Invalid(
                "upload_format_unsupported".into(),
                "批次包含不支持的表格格式".into(),
            ));
        }
        for file in &record.files {
            let source = self.data_dir.join(&file.staged_path);
            let ext = extension(&file.file_name);
            let outputs = match ext.as_str() {
                "csv" => {
                    let bytes = std::fs::read(&source)?;
                    if settings.delimiter.is_none() && ambiguous_delimiter(&bytes) {
                        needs_settings = true;
                        continue;
                    }
                    let out = self
                        .data_dir
                        .join("upload-work")
                        .join(record.batch.id.to_string())
                        .join(format!("{}.parquet", file.id));
                    normalize_csv_with_settings(
                        &source,
                        &out,
                        settings.delimiter.unwrap_or(b','),
                        settings.header_row,
                    )
                    .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?;
                    vec![(None, out)]
                }
                "xls" | "xlsx" => normalize_excel_to_parquet(
                    &source,
                    &self
                        .data_dir
                        .join("upload-work")
                        .join(record.batch.id.to_string())
                        .join(&file.id),
                )
                .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?
                .into_iter()
                .map(|s| (Some(s.sheet_name), s.path))
                .collect(),
                "parquet" => vec![(None, source.clone())],
                _ => {
                    return Err(UploadBatchError::Invalid(
                        "upload_format_unsupported".into(),
                        format!("不支持的上传格式: {}", file.file_name),
                    ));
                }
            };
            for (sheet, path) in outputs {
                let metadata = ParquetInspector::metadata(&path)
                    .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?;
                let inferred = self.infer_model(
                    &metadata
                        .columns
                        .iter()
                        .map(|c| c.name.clone())
                        .collect::<BTreeSet<_>>(),
                );
                let (alias, purpose, schema_ref) = match inferred {
                    Some((a, s)) => (Some(a), Some(CandidatePurpose::ModelTable), Some(s)),
                    None => {
                        needs_purpose = true;
                        (None, None, None)
                    }
                };
                let display = sheet.clone().unwrap_or_else(|| file.file_name.clone());
                parsed.push(ParsedTabularCandidate {
                    file_id: file.id.clone(),
                    sheet,
                    display,
                    path,
                    metadata,
                    alias,
                    purpose,
                    schema_ref,
                });
            }
        }
        let mut candidates = Vec::new();
        let mut committed = Vec::new();
        for parsed in parsed {
            let artifact = match self.commit_path(user.storage_scope_id, &parsed.path).await {
                Ok(value) => value,
                Err(error) => {
                    self.release_ids(user.storage_scope_id, &committed).await;
                    return Err(error);
                }
            };
            committed.push(artifact.id);
            candidates.push(candidate(record.batch.id,&parsed.file_id,parsed.sheet,&parsed.display,parsed.alias.as_deref(),parsed.purpose,parsed.schema_ref,artifact.id,&parsed.metadata,serde_json::json!({"source":"tabular","headerColumns":parsed.metadata.columns.iter().map(|c|&c.name).collect::<Vec<_>>() })));
        }
        let status = if needs_settings {
            "awaiting_parse_settings"
        } else if needs_purpose {
            "awaiting_purpose"
        } else {
            "ready_to_confirm"
        };
        if let Err(error) = self
            .repo
            .replace_candidates(
                record.batch.id,
                candidates,
                status,
                serde_json::to_string(settings).unwrap(),
                batch_diagnostics(record, settings, None).to_string(),
                now_ms(),
            )
            .await
        {
            self.release_ids(user.storage_scope_id, &committed).await;
            return Err(error.into());
        }
        self.release_previous(user.storage_scope_id, record).await;
        Ok(())
    }

    fn infer_model(&self, columns: &BTreeSet<String>) -> Option<(String, String)> {
        let mut matches = BTreeMap::<(usize, usize), (String, String)>::new();
        for facade in self.snapshot.facades.enabled() {
            for input in &facade.inputs {
                if input.kind != FacadeInputKind::Model {
                    continue;
                }
                if let Ok(schema) = self.snapshot.schemas.resolve(&input.schema_ref)
                    && schema.kind == DataFrameContractKind::Model
                {
                    let required = schema
                        .fields
                        .iter()
                        .filter(|f| f.required)
                        .map(|f| f.name.clone())
                        .collect::<BTreeSet<_>>();
                    if required.is_subset(columns) {
                        let known = schema
                            .fields
                            .iter()
                            .map(|f| f.name.as_str())
                            .collect::<BTreeSet<_>>();
                        let overlap = columns
                            .iter()
                            .filter(|column| known.contains(column.as_str()))
                            .count();
                        matches.insert(
                            (overlap, required.len()),
                            (input.alias.clone(), input.schema_ref.clone()),
                        );
                    }
                }
            }
        }
        matches.into_iter().next_back().map(|(_, value)| value)
    }

    fn materialize_tree(&self, record: &UploadBatchRecordSet) -> Result<PathBuf, UploadBatchError> {
        self.materialize_tree_at(record, "input")
    }

    fn materialize_case_tree(
        &self,
        record: &UploadBatchRecordSet,
    ) -> Result<PathBuf, UploadBatchError> {
        self.materialize_tree_at(record, "input.rssbundle")
    }

    fn materialize_tree_at(
        &self,
        record: &UploadBatchRecordSet,
        directory: &str,
    ) -> Result<PathBuf, UploadBatchError> {
        let root = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join(directory);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        for file in &record.files {
            let relative = file.relative_path.as_deref().unwrap_or(&file.file_name);
            let destination = root.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?
            }
            std::fs::copy(self.data_dir.join(&file.staged_path), destination)?;
        }
        Ok(root)
    }
    async fn commit_path(
        &self,
        scope: i32,
        path: &Path,
    ) -> Result<super::ArtifactRef, UploadBatchError> {
        let staged = self
            .artifacts
            .store()
            .stage(std::fs::File::open(path)?, StageLimits::default())
            .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?;
        Ok(self
            .artifacts
            .commit(scope, staged, ArtifactMetadata::parquet())
            .await?)
    }
    pub(super) async fn release_previous(&self, scope: i32, record: &UploadBatchRecordSet) {
        for artifact in record
            .candidates
            .iter()
            .filter_map(|candidate| candidate.normalized_artifact_id)
        {
            let _ = self.artifacts.release(scope, artifact).await;
        }
    }
    async fn release_ids(&self, scope: i32, artifacts: &[i32]) {
        for artifact in artifacts {
            let _ = self.artifacts.release(scope, *artifact).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    batch: i32,
    file: &str,
    sheet: Option<String>,
    display: &str,
    alias: Option<&str>,
    purpose: Option<CandidatePurpose>,
    schema_ref: Option<String>,
    artifact: i32,
    metadata: &crate::storage::ParquetMetadata,
    evidence: serde_json::Value,
) -> NewUploadCandidate {
    let schema = serde_json::to_value(&metadata.columns).unwrap();
    let schema_json = schema.to_string();
    NewUploadCandidate {
        id: stable_id(
            "candidate",
            &format!("{batch}:{file}:{}", sheet.as_deref().unwrap_or(display)),
        ),
        batch_id: batch,
        file_id: file.into(),
        sheet_name: sheet,
        display_name: display.into(),
        proposed_alias: alias.map(str::to_string),
        normalized_alias: alias.map(normalize_alias),
        alias_explicit: false,
        purpose: purpose.map(|p| p.as_str().into()),
        inferred_schema_ref: schema_ref,
        normalized_artifact_id: Some(artifact),
        schema_hash: Some(hex::encode(Sha256::digest(schema_json.as_bytes()))),
        schema_json: Some(schema_json),
        row_count: i64::try_from(metadata.row_count).ok(),
        evidence_json: evidence.to_string(),
        selected: true,
        created_at_ms: now_ms(),
    }
}
fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn batch_diagnostics(
    record: &UploadBatchRecordSet,
    settings: &ParseSettings,
    error: Option<&UploadBatchError>,
) -> serde_json::Value {
    let source_hash = source_fingerprint(record.files.iter().map(|file| {
        (
            &file.client_file_id,
            &file.file_name,
            file.relative_path.as_deref(),
            file.sha256.as_str(),
        )
    }));
    serde_json::json!({"sourceHash":source_hash,"parserVersion":"v4.1","parseSpec":settings,"errors":error.map(|value|vec![serde_json::json!({"code":value.code(),"message":value.to_string()})]).unwrap_or_default()})
}
fn ambiguous_delimiter(bytes: &[u8]) -> bool {
    let first = String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    [',', ';', '\t']
        .into_iter()
        .filter(|c| first.contains(*c))
        .count()
        > 1
}

fn read_projection_diagnostics(
    datasets: &[crate::adapters::EngineDatasetOutput],
) -> Result<Vec<serde_json::Value>, UploadBatchError> {
    const MAX_ROWS: u64 = 100;
    const MAX_BYTES: u64 = 1024 * 1024;
    let mut output = Vec::with_capacity(datasets.len());
    for dataset in datasets {
        if dataset.metadata.row_count > MAX_ROWS || dataset.metadata.size_bytes > MAX_BYTES {
            return Err(UploadBatchError::Invalid(
                "case_projection_diagnostic_limit".into(),
                "案例投影诊断超过 100 行或 1 MiB 限制".into(),
            ));
        }
        let batches = if dataset.metadata.row_count == 0 {
            Vec::new()
        } else {
            ParquetInspector::page(&dataset.path, 0, MAX_ROWS as usize).map_err(|error| {
                UploadBatchError::Invalid(error.code().into(), error.to_string())
            })?
        };
        let mut conversion_diagnostics = BTreeMap::new();
        let mut rows = Vec::new();
        for batch in batches {
            for row_index in 0..batch.num_rows() {
                let mut row = serde_json::Map::new();
                for (column_index, field) in batch.schema().fields().iter().enumerate() {
                    let value = super::dataframe_reader::value(
                        batch.column(column_index).as_ref(),
                        row_index,
                        &mut conversion_diagnostics,
                    )
                    .map_err(|error| {
                        UploadBatchError::Invalid(
                            "case_projection_diagnostic_invalid".into(),
                            error.to_string(),
                        )
                    })?;
                    row.insert(field.name().clone(), value);
                }
                rows.push(serde_json::Value::Object(row));
            }
        }
        output.push(serde_json::json!({
            "name": dataset.name,
            "rowCount": dataset.metadata.row_count,
            "schema": dataset.metadata.columns,
            "rows": rows,
            "conversionDiagnostics": conversion_diagnostics
        }));
    }
    Ok(output)
}
