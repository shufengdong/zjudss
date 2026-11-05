use super::{ChartServiceError, CreateChartViewRequest};
use crate::application::charting::{
    CHART_CONTRACT_VERSION, CompiledChart, NormalizedCreateChart, PlotDataCodec,
    normalize_create_request,
};
use crate::db::repositories::{ArtifactDraft, ChartViewDraft};
use crate::storage::{ArtifactStore, StagedArtifact};

pub(super) struct PreparedRequest {
    pub client_operation_id: String,
    pub normalized: NormalizedCreateChart,
}

pub(super) struct PreparedChartView {
    pub draft: ChartViewDraft,
    pub staged: StagedArtifact,
}

pub(super) fn prepare_request(
    request: CreateChartViewRequest,
) -> Result<PreparedRequest, ChartServiceError> {
    validate_operation_id(&request.client_operation_id)?;
    Ok(PreparedRequest {
        client_operation_id: request.client_operation_id,
        normalized: normalize_create_request(request.name, request.config)?,
    })
}

pub(super) fn prepare_materialized_view(
    store: &ArtifactStore,
    dataset_id: i32,
    request: PreparedRequest,
    compiled: &CompiledChart,
) -> Result<PreparedChartView, ChartServiceError> {
    let materialized = PlotDataCodec::write_snappy(store, compiled)?;
    let diagnostics_json = serde_json::to_string(&compiled.diagnostics)
        .map_err(|error| ChartServiceError::Stored(error.to_string()))?;
    let row_count = checked_i64(materialized.row_count)?;
    let point_count = checked_i64(materialized.point_count)?;
    let size_bytes = checked_i64(materialized.size_bytes)?;
    let created_at_ms = crate::application::now_ms();
    let normalized = request.normalized;
    let draft = ChartViewDraft {
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
        created_at_ms,
        artifact: ArtifactDraft {
            sha256: materialized.content_hash,
            size_bytes,
            storage_path: String::new(),
            format: "parquet".into(),
            media_type: Some("application/vnd.apache.parquet".into()),
            created_at_ms,
        },
    };
    Ok(PreparedChartView {
        draft,
        staged: materialized.staged,
    })
}

fn checked_i64(value: u64) -> Result<i64, ChartServiceError> {
    i64::try_from(value).map_err(|error| ChartServiceError::Stored(error.to_string()))
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
        Err(crate::application::charting::ChartError::new(
            "chart_config_invalid",
            "clientOperationId 必须为 UUID",
        )
        .into())
    }
}
