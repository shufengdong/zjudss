use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;
use crate::application::artifacts::{ArtifactError, ArtifactService};
use crate::application::dataframe_reader::{DataFrameColumn, ParquetDataFrameReader};
use crate::application::result_sources::ResultSourceService;
use crate::auth::CurrentUser;
use crate::db::repositories::{ChartViewRepo, RepoError, ResultRepo};
use crate::storage::ParquetInspector;

mod materialization;
mod presentation;
mod source;

use materialization::{prepare_materialized_view, prepare_request};
use presentation::{decode_cursor, encode_cursor, frozen_model, metadata, summary};
use source::{compile_source, map_source_artifact_error, resolve_source};

#[derive(Debug)]
pub enum ChartServiceError {
    Chart(ChartError),
    Repository(RepoError),
    Artifact(ArtifactError),
    InvalidCursor,
    Stored(String),
}

impl ChartServiceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Chart(error) => error.code,
            Self::Repository(RepoError::NotFound) | Self::Artifact(ArtifactError::NotFound) => {
                "not_found"
            }
            Self::Repository(RepoError::IdempotencyConflict) => "chart_operation_conflict",
            Self::Repository(RepoError::InvalidState(code)) if code == "chart_name_conflict" => {
                "chart_name_conflict"
            }
            Self::Repository(RepoError::InvalidState(code)) if code == "chart_operation_gone" => {
                "chart_operation_gone"
            }
            Self::Repository(RepoError::QuotaExceeded)
            | Self::Artifact(ArtifactError::QuotaExceeded) => "storage_quota_exceeded",
            Self::InvalidCursor => "chart_cursor_invalid",
            Self::Stored(_) => "chart_materialization_failed",
            _ => "chart_materialization_failed",
        }
    }

    pub fn details(&self) -> Option<Value> {
        match self {
            Self::Chart(error) => error.details.clone(),
            _ => None,
        }
    }
}

impl std::fmt::Display for ChartServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chart(error) => write!(formatter, "{error}"),
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::InvalidCursor => write!(formatter, "图表分页 cursor 无效"),
            Self::Stored(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ChartServiceError {}

impl From<ChartError> for ChartServiceError {
    fn from(value: ChartError) -> Self {
        Self::Chart(value)
    }
}
impl From<RepoError> for ChartServiceError {
    fn from(value: RepoError) -> Self {
        Self::Repository(value)
    }
}
impl From<ArtifactError> for ChartServiceError {
    fn from(value: ArtifactError) -> Self {
        Self::Artifact(value)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartPreviewRequest {
    pub config: ChartConfig,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateChartViewRequest {
    pub client_operation_id: String,
    pub name: String,
    pub config: ChartConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartViewSummary {
    pub id: String,
    pub session_id: String,
    pub source_dataset_id: String,
    pub source_name: String,
    pub source_deleted: bool,
    pub name: String,
    pub kind: ChartKind,
    pub point_count: i64,
    pub size_bytes: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartViewMetadata {
    #[serde(flatten)]
    pub summary: ChartViewSummary,
    pub contract_version: String,
    pub config: ChartConfig,
    pub config_hash: String,
    pub source_schema: Vec<DataFrameColumn>,
    pub source_schema_hash: String,
    pub source_content_hash: String,
    pub plot_schema: Vec<DataFrameColumn>,
    pub plot_schema_hash: String,
    pub plot_content_hash: String,
    pub diagnostics: ChartDiagnostics,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChartViewResponse {
    pub view: ChartViewMetadata,
    pub replayed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartViewPage {
    pub views: Vec<ChartViewSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteChartViewResponse {
    pub view_id: String,
    pub tombstone: Value,
}

#[derive(Clone)]
pub struct ChartService {
    sources: ResultSourceService,
    views: ChartViewRepo,
    artifacts: ArtifactService,
    reader: ParquetDataFrameReader,
}

impl ChartService {
    pub fn new(results: ResultRepo, views: ChartViewRepo, artifacts: ArtifactService) -> Self {
        Self {
            sources: ResultSourceService::new(results, artifacts.clone()),
            views,
            reader: ParquetDataFrameReader::new(artifacts.clone()),
            artifacts,
        }
    }

    pub async fn capabilities(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
    ) -> Result<ChartCapabilities, ChartServiceError> {
        let source = resolve_source(&self.sources, principal, dataset_id).await?;
        let budgets = ChartBudgets::default();
        if source.columns.len() > budgets.max_fields {
            return Err(ChartError::new(
                "chart_capabilities_too_large",
                "DataFrame 列数超过 capability 预算",
            )
            .with_details(serde_json::json!({
                "observed": source.columns.len(),
                "limit": budgets.max_fields
            }))
            .into());
        }
        let file = self
            .artifacts
            .open_owned(principal.storage_scope_id, source.artifact.id)
            .await
            .map_err(map_source_artifact_error)?;
        let batches = ParquetInspector::projection_file(
            file,
            &source
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>(),
            0,
            CHART_CAPABILITY_SCAN_ROWS,
        )
        .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        CapabilityInspector::inspect(
            dataset_id.to_string(),
            source.dataset.schema_hash,
            &source.columns,
            &batches,
            source.dataset.row_count as u64,
            budgets,
        )
        .map_err(Into::into)
    }

    pub async fn preview(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
        config: ChartConfig,
    ) -> Result<ChartModel, ChartServiceError> {
        Ok(self.compile(principal, dataset_id, &config).await?.model)
    }

    pub async fn save(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
        request: CreateChartViewRequest,
    ) -> Result<CreateChartViewResponse, ChartServiceError> {
        let prepared_request = prepare_request(request)?;
        if let Some(view) = self
            .views
            .find_operation(
                &principal.id,
                dataset_id,
                &prepared_request.client_operation_id,
            )
            .await?
        {
            if view.deleted_at_ms.is_some() {
                return Err(RepoError::InvalidState("chart_operation_gone".into()).into());
            }
            if view.request_hash != prepared_request.normalized.request_hash {
                return Err(RepoError::IdempotencyConflict.into());
            }
            let record = self.views.find(&principal.id, view.id).await?;
            return Ok(CreateChartViewResponse {
                view: metadata(record.view, record.source_deleted)?,
                replayed: true,
            });
        }
        let source = resolve_source(&self.sources, principal, dataset_id).await?;
        let compiled = compile_source(
            &self.reader,
            principal,
            &source,
            &prepared_request.normalized.config,
        )
        .await?;
        let prepared = prepare_materialized_view(
            self.artifacts.store(),
            dataset_id,
            prepared_request,
            &compiled,
        )?;
        let published = self
            .artifacts
            .publish_chart_view(
                self.views.clone(),
                principal.id.clone(),
                prepared.draft,
                prepared.staged,
            )
            .await
            .map_err(|error| match error {
                ArtifactError::Repository(RepoError::NotFound) => ChartServiceError::Chart(
                    ChartError::new("chart_source_deleted", "图表发布前来源结果已被删除"),
                ),
                other => ChartServiceError::Artifact(other),
            })?;
        let record = self.views.find(&principal.id, published.view.id).await?;
        Ok(CreateChartViewResponse {
            view: metadata(record.view, record.source_deleted)?,
            replayed: published.replayed,
        })
    }

    pub async fn metadata(
        &self,
        principal: &CurrentUser,
        id: i32,
    ) -> Result<ChartViewMetadata, ChartServiceError> {
        let record = self.views.find(&principal.id, id).await?;
        metadata(record.view, record.source_deleted)
    }

    pub async fn data(
        &self,
        principal: &CurrentUser,
        id: i32,
    ) -> Result<ChartModel, ChartServiceError> {
        let record = self.views.find(&principal.id, id).await?;
        let frozen = frozen_model(&record.view, record.source_deleted)?;
        let file = self
            .artifacts
            .open_owned(
                principal.storage_scope_id,
                record.view.artifact_id.ok_or(RepoError::NotFound)?,
            )
            .await
            .map_err(|error| match error {
                ArtifactError::NotFound => {
                    ChartServiceError::Stored("已保存图表的 Plot Data artifact 不存在".into())
                }
                other => ChartServiceError::Artifact(other),
            })?;
        PlotDataCodec::read_model(file, &frozen).map_err(Into::into)
    }

    pub async fn list(
        &self,
        principal: &CurrentUser,
        session_id: i32,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ChartViewPage, ChartServiceError> {
        if !(1..=100).contains(&limit) {
            return Err(ChartServiceError::InvalidCursor);
        }
        let before = cursor
            .map(|cursor| decode_cursor(cursor, principal, session_id))
            .transpose()?;
        let rows = self
            .views
            .list(
                &principal.id,
                session_id,
                before,
                i64::try_from(limit + 1).expect("limit is bounded"),
            )
            .await?;
        let has_more = rows.len() > limit;
        let visible = rows.into_iter().take(limit).collect::<Vec<_>>();
        let next_cursor = has_more
            .then(|| visible.last())
            .flatten()
            .map(|row| encode_cursor(principal, session_id, row.created_at_ms, row.id));
        Ok(ChartViewPage {
            views: visible.into_iter().map(summary).collect::<Result<_, _>>()?,
            next_cursor,
        })
    }

    pub async fn delete(
        &self,
        principal: &CurrentUser,
        id: i32,
    ) -> Result<DeleteChartViewResponse, ChartServiceError> {
        let deleted = self
            .views
            .delete(&principal.id, id, crate::application::now_ms())
            .await?;
        self.artifacts.collect_unreferenced().await?;
        let mut tombstone: Value = serde_json::from_str(&deleted.tombstone_json)
            .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        if let Some(timestamp) = tombstone.get("deletedAt").and_then(Value::as_i64) {
            tombstone["deletedAt"] = Value::String(crate::application::ms_to_iso(timestamp));
        }
        Ok(DeleteChartViewResponse {
            view_id: deleted.view_id.to_string(),
            tombstone,
        })
    }

    async fn compile(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
        config: &ChartConfig,
    ) -> Result<CompiledChart, ChartServiceError> {
        let source = resolve_source(&self.sources, principal, dataset_id).await?;
        compile_source(&self.reader, principal, &source, config).await
    }
}
