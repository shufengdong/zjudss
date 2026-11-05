use std::collections::HashSet;

use super::ChartServiceError;
use crate::application::artifacts::ArtifactError;
use crate::application::charting::{
    ChartBudgets, ChartCompiler, ChartConfig, ChartError, ChartFieldRef, ChartSource, CompiledChart,
};
use crate::application::dataframe_reader::{DataFrameReadError, ParquetDataFrameReader};
use crate::application::result_sources::{ResultSource, ResultSourceError, ResultSourceService};
use crate::auth::CurrentUser;
use crate::db::repositories::RepoError;

pub(super) async fn resolve_source(
    sources: &ResultSourceService,
    principal: &CurrentUser,
    dataset_id: i32,
) -> Result<ResultSource, ChartServiceError> {
    sources
        .resolve(principal, dataset_id)
        .await
        .map_err(map_result_source_error)
}

pub(super) async fn compile_source(
    reader: &ParquetDataFrameReader,
    principal: &CurrentUser,
    source: &ResultSource,
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
    let batches = reader
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
            source.dataset.id.to_string(),
            source.columns.clone(),
            batches,
        ),
    )
    .map_err(Into::into)
}

pub(super) fn map_result_source_error(error: ResultSourceError) -> ChartServiceError {
    match error {
        ResultSourceError::NotFound => ChartServiceError::Repository(RepoError::NotFound),
        ResultSourceError::Repository(error) => ChartServiceError::Repository(error),
        ResultSourceError::Artifact(ArtifactError::NotFound) => source_deleted_error(),
        ResultSourceError::Artifact(error) => ChartServiceError::Artifact(error),
        ResultSourceError::Stored(message) => ChartServiceError::Stored(message),
    }
}

pub(super) fn map_source_artifact_error(error: ArtifactError) -> ChartServiceError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::result_sources::ResultSourceError;

    #[test]
    fn result_source_errors_preserve_chart_operation_semantics() {
        assert_eq!(
            map_result_source_error(ResultSourceError::NotFound).code(),
            "not_found"
        );
        assert_eq!(
            map_result_source_error(ResultSourceError::Artifact(
                crate::application::artifacts::ArtifactError::NotFound
            ))
            .code(),
            "chart_source_deleted"
        );
        assert_eq!(
            map_result_source_error(ResultSourceError::Stored("bad schema".into())).code(),
            "chart_materialization_failed"
        );
    }
}
