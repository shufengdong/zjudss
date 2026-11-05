use serde_json::Value;

use super::{ChartServiceError, ChartViewMetadata, ChartViewSummary};
use crate::application::charting::*;
use crate::application::dataframe_reader::DataFrameColumn;
use crate::auth::CurrentUser;
use crate::db::models::ChartViewRow;
use crate::db::repositories::ChartViewSummaryRow;

pub(super) fn metadata(
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

pub(super) fn summary(row: ChartViewSummaryRow) -> Result<ChartViewSummary, ChartServiceError> {
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

pub(super) fn frozen_model(
    row: &ChartViewRow,
    source_deleted: bool,
) -> Result<ChartModel, ChartServiceError> {
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

pub(super) fn encode_cursor(
    principal: &CurrentUser,
    session_id: i32,
    created_at: i64,
    id: i32,
) -> String {
    hex::encode(format!(
        "v1|{}|{session_id}|{created_at}|{id}",
        principal.id.0
    ))
}

pub(super) fn decode_cursor(
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
