use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::*;
use crate::application::artifacts::{ArtifactError, ArtifactService};
use crate::application::dataframe_reader::{
    DataFrameColumn, DataFrameReadError, DataFrameReadSource, ParquetDataFrameReader,
};
use crate::auth::CurrentUser;
use crate::db::models::ChartViewRow;
use crate::db::repositories::{
    ArtifactDraft, ChartViewDraft, ChartViewRepo, ChartViewSummaryRow, RepoError, ResultRepo,
};
use crate::storage::ParquetInspector;

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
    results: ResultRepo,
    views: ChartViewRepo,
    artifacts: ArtifactService,
    reader: ParquetDataFrameReader,
}

impl ChartService {
    pub fn new(results: ResultRepo, views: ChartViewRepo, artifacts: ArtifactService) -> Self {
        Self {
            results,
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
        let source = self.source(principal, dataset_id).await?;
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
            .open_owned(principal.storage_scope_id, source.artifact_id)
            .await
            .map_err(source_artifact_error)?;
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
            source.schema_hash,
            &source.columns,
            &batches,
            source.row_count as u64,
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
        validate_operation_id(&request.client_operation_id)?;
        let normalized = normalize_create_request(request.name, request.config)?;
        if let Some(view) = self
            .views
            .find_operation(&principal.id, dataset_id, &request.client_operation_id)
            .await?
        {
            if view.deleted_at_ms.is_some() {
                return Err(RepoError::InvalidState("chart_operation_gone".into()).into());
            }
            if view.request_hash != normalized.request_hash {
                return Err(RepoError::IdempotencyConflict.into());
            }
            let record = self.views.find(&principal.id, view.id).await?;
            return Ok(CreateChartViewResponse {
                view: metadata(record.view, record.source_deleted)?,
                replayed: true,
            });
        }
        let source = self.source(principal, dataset_id).await?;
        let compiled = self
            .compile_source(principal, &source, &normalized.config)
            .await?;
        let materialized = PlotDataCodec::write_snappy(self.artifacts.store(), &compiled)?;
        let diagnostics_json = serde_json::to_string(&compiled.diagnostics)
            .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        let row_count = i64::try_from(materialized.row_count)
            .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        let point_count = i64::try_from(materialized.point_count)
            .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        let size_bytes = i64::try_from(materialized.size_bytes)
            .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        let published = self
            .artifacts
            .publish_chart_view(
                self.views.clone(),
                principal.id.clone(),
                ChartViewDraft {
                    source_dataset_id: dataset_id,
                    client_operation_id: request.client_operation_id,
                    request_hash: normalized.request_hash,
                    name: normalized.name,
                    kind: normalized.config.kind().as_str().into(),
                    config_json: normalized.config_json,
                    config_hash: normalized.config_hash,
                    chart_contract_version: CHART_CONTRACT_VERSION.into(),
                    diagnostics_json,
                    plot_schema_json: materialized.schema_json,
                    plot_schema_hash: materialized.schema_hash,
                    plot_content_hash: materialized.content_hash.clone(),
                    row_count,
                    point_count,
                    size_bytes,
                    created_at_ms: crate::application::now_ms(),
                    artifact: ArtifactDraft {
                        sha256: materialized.content_hash,
                        size_bytes,
                        storage_path: String::new(),
                        format: "parquet".into(),
                        media_type: Some("application/vnd.apache.parquet".into()),
                        created_at_ms: crate::application::now_ms(),
                    },
                },
                materialized.staged,
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
        let source = self.source(principal, dataset_id).await?;
        self.compile_source(principal, &source, config).await
    }

    async fn compile_source(
        &self,
        principal: &CurrentUser,
        source: &Source,
        config: &ChartConfig,
    ) -> Result<CompiledChart, ChartServiceError> {
        let columns = referenced_columns(config);
        if let Some(missing) = columns
            .iter()
            .find(|name| !source.columns.iter().any(|column| column.name == ***name))
        {
            return Err(ChartError::new("chart_column_missing", "图表字段不存在")
                .with_details(serde_json::json!({"column": missing}))
                .into());
        }
        let batches = self
            .reader
            .collect_projected_batches(
                principal,
                &source.read_source,
                &columns,
                ChartBudgets::default().max_scan_rows as usize,
            )
            .await
            .map_err(|error| match error {
                DataFrameReadError::TooManyRows(limit) => ChartServiceError::Chart(
                    ChartError::new("chart_too_large", "DataFrame 扫描行数超过图表预算")
                        .with_details(serde_json::json!({"limit": limit})),
                ),
                DataFrameReadError::NotFound => source_deleted_error(),
                other => ChartServiceError::Stored(other.to_string()),
            })?;
        ChartCompiler::compile(
            config,
            &ChartSource::new(
                source.dataset_id.to_string(),
                source.columns.clone(),
                batches,
            ),
        )
        .map_err(Into::into)
    }

    async fn source(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
    ) -> Result<Source, ChartServiceError> {
        let dataset = self.results.find_live(&principal.id, dataset_id).await?;
        let artifact_id = dataset.artifact_id.ok_or(RepoError::NotFound)?;
        let artifact = self
            .artifacts
            .find_owned(principal.storage_scope_id, artifact_id)
            .await
            .map_err(source_artifact_error)?;
        let columns = serde_json::from_str::<Vec<DataFrameColumn>>(&dataset.schema_json)
            .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
        Ok(Source {
            dataset_id,
            artifact_id,
            schema_hash: dataset.schema_hash.clone(),
            row_count: dataset.row_count,
            columns,
            read_source: DataFrameReadSource {
                resource_key: format!("dataset:{dataset_id}"),
                artifact_id,
                schema_json: dataset.schema_json,
                row_count: dataset.row_count,
                content_hash: artifact.sha256,
            },
        })
    }
}

fn source_artifact_error(error: ArtifactError) -> ChartServiceError {
    match error {
        ArtifactError::NotFound => source_deleted_error(),
        other => ChartServiceError::Artifact(other),
    }
}

fn source_deleted_error() -> ChartServiceError {
    ChartServiceError::Chart(ChartError::new(
        "chart_source_deleted",
        "首次校验后来源结果已被删除",
    ))
}

struct Source {
    dataset_id: i32,
    artifact_id: i32,
    schema_hash: String,
    row_count: i64,
    columns: Vec<DataFrameColumn>,
    read_source: DataFrameReadSource,
}

fn referenced_columns(config: &ChartConfig) -> Vec<String> {
    let fields: Vec<&ChartFieldRef> = match config {
        ChartConfig::Line {
            x,
            measures,
            split_by,
            ..
        }
        | ChartConfig::Bar {
            x,
            measures,
            split_by,
            ..
        }
        | ChartConfig::Scatter {
            x,
            measures,
            split_by,
            ..
        } => std::iter::once(x)
            .chain(measures.iter())
            .chain(split_by.iter())
            .collect(),
        ChartConfig::Heatmap { x, y, value, .. } => vec![x, y, value],
        ChartConfig::Boxplot {
            category,
            value,
            split_by,
            ..
        } => std::iter::once(category)
            .chain(std::iter::once(value))
            .chain(split_by.iter())
            .collect(),
        ChartConfig::Histogram { value, .. } => vec![value],
    };
    let mut seen = HashSet::new();
    fields
        .into_iter()
        .map(|field| field.column.clone())
        .filter(|column| seen.insert(column.clone()))
        .collect()
}

fn metadata(
    row: ChartViewRow,
    source_deleted: bool,
) -> Result<ChartViewMetadata, ChartServiceError> {
    let config = serde_json::from_str::<ChartConfig>(&row.config_json)
        .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
    Ok(ChartViewMetadata {
        summary: summary(ChartViewSummaryRow {
            id: row.id,
            session_id: row.session_id,
            source_dataset_id: row.source_dataset_id,
            source_name: row.source_name.clone(),
            source_deleted,
            name: row.name.clone(),
            kind: row.kind.clone(),
            point_count: row.point_count,
            size_bytes: row.size_bytes,
            created_at_ms: row.created_at_ms,
        })?,
        contract_version: row.chart_contract_version,
        config,
        config_hash: row.config_hash,
        source_schema: parse_json(&row.source_schema_json)?,
        source_schema_hash: row.source_schema_hash,
        source_content_hash: row.source_content_hash,
        plot_schema: parse_json(&row.plot_schema_json)?,
        plot_schema_hash: row.plot_schema_hash,
        plot_content_hash: row.plot_content_hash,
        diagnostics: parse_json(&row.diagnostics_json)?,
    })
}

fn summary(row: ChartViewSummaryRow) -> Result<ChartViewSummary, ChartServiceError> {
    let kind = serde_json::from_value(Value::String(row.kind))
        .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
    Ok(ChartViewSummary {
        id: row.id.to_string(),
        session_id: row.session_id.to_string(),
        source_dataset_id: row.source_dataset_id.to_string(),
        source_name: row.source_name,
        source_deleted: row.source_deleted,
        name: row.name,
        kind,
        point_count: row.point_count,
        size_bytes: row.size_bytes,
        created_at: crate::application::ms_to_iso(row.created_at_ms),
    })
}

fn frozen_model(row: &ChartViewRow, source_deleted: bool) -> Result<ChartModel, ChartServiceError> {
    let config: ChartConfig = parse_json(&row.config_json)?;
    let schema: Vec<DataFrameColumn> = parse_json(&row.source_schema_json)?;
    let diagnostics = parse_json(&row.diagnostics_json)?;
    let base = ChartModelBase {
        contract_version: row.chart_contract_version.clone(),
        view_id: Some(row.id.to_string()),
        source_dataset_id: row.source_dataset_id.to_string(),
        source_deleted,
        name: Some(row.name.clone()),
        title: config_title(&config),
        config_hash: row.config_hash.clone(),
        diagnostics,
    };
    let unit = |field: &ChartFieldRef| {
        schema
            .iter()
            .find(|column| column.name == field.column)
            .and_then(|column| column.unit.clone())
    };
    Ok(match config {
        ChartConfig::Line {
            presentation,
            x,
            x_axis_mode,
            measures,
            order,
            aggregation,
            ..
        } => ChartModel::Line {
            base,
            presentation,
            order,
            aggregation,
            x_axis: MixedAxis {
                r#type: x_axis_mode,
                name: x.display_name(),
                unit: unit(&x),
            },
            y_axis: ValueAxis {
                name: (measures.len() == 1).then(|| measures[0].display_name()),
                unit: unit(&measures[0]),
            },
            series: Vec::new(),
        },
        ChartConfig::Bar {
            presentation,
            x,
            measures,
            ..
        } => ChartModel::Bar {
            base,
            presentation,
            x_axis: CategoryAxis {
                r#type: CategoryAxisType::Category,
                name: x.display_name(),
                unit: unit(&x),
            },
            y_axis: ValueAxis {
                name: (measures.len() == 1).then(|| measures[0].display_name()),
                unit: unit(&measures[0]),
            },
            categories: Vec::new(),
            series: Vec::new(),
        },
        ChartConfig::Scatter {
            presentation,
            x,
            measures,
            ..
        } => ChartModel::Scatter {
            base,
            presentation,
            x_axis: MixedAxis {
                r#type: AxisMode::Value,
                name: x.display_name(),
                unit: unit(&x),
            },
            y_axis: ValueAxis {
                name: (measures.len() == 1).then(|| measures[0].display_name()),
                unit: unit(&measures[0]),
            },
            series: Vec::new(),
        },
        ChartConfig::Heatmap {
            presentation,
            x,
            y,
            value,
            aggregation,
            ..
        } => ChartModel::Heatmap {
            base,
            presentation,
            x_axis: NamedCategoryAxis {
                name: x.display_name(),
                categories: Vec::new(),
            },
            y_axis: NamedCategoryAxis {
                name: y.display_name(),
                categories: Vec::new(),
            },
            x_categories: Vec::new(),
            y_categories: Vec::new(),
            value: HeatmapValue {
                name: value.display_name(),
                unit: (aggregation != HeatmapAggregation::Count)
                    .then(|| unit(&value))
                    .flatten(),
                min: 0.0,
                max: 0.0,
            },
            cells: Vec::new(),
        },
        ChartConfig::Boxplot {
            presentation,
            category,
            value,
            ..
        } => ChartModel::Boxplot {
            base,
            presentation,
            category_axis: NamedCategoryAxis {
                name: category.display_name(),
                categories: Vec::new(),
            },
            value_axis: NamedValueAxis {
                name: value.display_name(),
                unit: unit(&value),
            },
            series: Vec::new(),
        },
        ChartConfig::Histogram {
            presentation,
            value,
            ..
        } => ChartModel::Histogram {
            base,
            presentation,
            value: HistogramValue {
                name: value.display_name(),
                unit: unit(&value),
            },
            bins: Vec::new(),
        },
    })
}

fn config_title(config: &ChartConfig) -> Option<String> {
    match config {
        ChartConfig::Line { title, .. }
        | ChartConfig::Bar { title, .. }
        | ChartConfig::Scatter { title, .. }
        | ChartConfig::Heatmap { title, .. }
        | ChartConfig::Boxplot { title, .. }
        | ChartConfig::Histogram { title, .. } => title.clone(),
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, ChartServiceError> {
    serde_json::from_str(value).map_err(|error| ChartServiceError::Stored(error.to_string()))
}

fn validate_operation_id(value: &str) -> Result<(), ChartServiceError> {
    let valid = value.len() == 36
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| match index {
                8 | 13 | 18 | 23 => character == '-',
                _ => character.is_ascii_hexdigit(),
            });
    if valid {
        Ok(())
    } else {
        Err(ChartError::new("chart_config_invalid", "clientOperationId 必须为 UUID").into())
    }
}

fn encode_cursor(principal: &CurrentUser, session_id: i32, created_at: i64, id: i32) -> String {
    hex::encode(format!(
        "v1|{}|{session_id}|{created_at}|{id}",
        principal.id.0
    ))
}

fn decode_cursor(
    cursor: &str,
    principal: &CurrentUser,
    session_id: i32,
) -> Result<(i64, i32), ChartServiceError> {
    let bytes = hex::decode(cursor).map_err(|_| ChartServiceError::InvalidCursor)?;
    let value = String::from_utf8(bytes).map_err(|_| ChartServiceError::InvalidCursor)?;
    let parts = value.split('|').collect::<Vec<_>>();
    if parts.len() != 5
        || parts[0] != "v1"
        || parts[1] != principal.id.0
        || parts[2] != session_id.to_string()
    {
        return Err(ChartServiceError::InvalidCursor);
    }
    Ok((
        parts[3]
            .parse()
            .map_err(|_| ChartServiceError::InvalidCursor)?,
        parts[4]
            .parse()
            .map_err(|_| ChartServiceError::InvalidCursor)?,
    ))
}
