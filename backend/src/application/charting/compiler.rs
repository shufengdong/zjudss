use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use arrow_array::RecordBatch;

use super::contracts::normalize_chart_config;
use super::{
    Aggregation, AxisMode, BarSeries, BoxplotOutlier, BoxplotSeries, CHART_CONTRACT_VERSION,
    CategoryAxis, CategoryAxisType, CategoryValue, ChartBudgets, ChartConfig, ChartDiagnostics,
    ChartError, ChartFieldRef, ChartModel, ChartModelBase, ChartOrder, ChartPresentation,
    ChartSampling, HeatmapAggregation, HeatmapCell, HeatmapValue, HistogramBin, HistogramValue,
    LineChartPresentation, LinePoint, LineSeries, LineXValue, MixedAxis, NamedCategoryAxis,
    NamedValueAxis, NumericPoint, ResolvedField, ScatterSeries, ValueAxis, lttb,
};
use crate::application::dataframe_reader::DataFrameColumn;

mod bar;
mod boxplot;
mod common;
mod heatmap;
mod histogram;
mod line;
mod scatter;
mod schema;
mod source;

use common::*;
use schema::*;
pub use source::ChartSource;

#[derive(Debug, Clone, PartialEq)]
pub enum PlotData {
    Line(Vec<PlotLineRow>),
    Bar(Vec<PlotBarRow>),
    Scatter(Vec<PlotScatterRow>),
    Heatmap(Vec<PlotHeatmapRow>),
    Boxplot(Vec<PlotBoxplotRow>),
    Histogram(Vec<PlotHistogramRow>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotLineRow {
    pub ordinal: u64,
    pub x: LineXValue,
    pub series: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotBarRow {
    pub category_index: usize,
    pub category: CategoryValue,
    pub series: String,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotScatterRow {
    pub ordinal: u64,
    pub series: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotHeatmapRow {
    pub x_index: usize,
    pub x_category: CategoryValue,
    pub y_index: usize,
    pub y_category: CategoryValue,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotBoxplotRow {
    pub category_index: usize,
    pub category: CategoryValue,
    pub series: String,
    pub values: [f64; 5],
    pub outliers: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlotHistogramRow {
    pub bin_index: usize,
    pub start: f64,
    pub end: f64,
    pub count: u64,
    pub includes_end: bool,
}

#[derive(Debug, Clone)]
pub struct CompiledChart {
    pub model: ChartModel,
    pub plot_data: PlotData,
    pub plot_schema: Vec<DataFrameColumn>,
    pub diagnostics: ChartDiagnostics,
}

pub struct ChartCompiler;

impl ChartCompiler {
    pub fn compile(
        config: &ChartConfig,
        source: &ChartSource,
    ) -> Result<CompiledChart, ChartError> {
        Self::compile_with_budgets(config, source, ChartBudgets::default())
    }

    pub fn compile_with_budgets(
        config: &ChartConfig,
        source: &ChartSource,
        budgets: ChartBudgets,
    ) -> Result<CompiledChart, ChartError> {
        let (config, _, config_hash) = normalize_chart_config(config.clone())?;
        let referenced = referenced_fields(&config);
        let unique_columns = referenced
            .iter()
            .map(|field| field.column.as_str())
            .collect::<HashSet<_>>();
        enforce_budget(
            "referencedColumns",
            unique_columns.len(),
            budgets.max_referenced_columns,
        )?;
        let input_rows = source
            .batches
            .iter()
            .map(RecordBatch::num_rows)
            .sum::<usize>();
        enforce_budget("scanRows", input_rows, budgets.max_scan_rows as usize)?;
        let first = source.batches.first().ok_or_else(no_data)?;
        let resolved = referenced
            .iter()
            .map(|field| ResolvedField::bind(&source.columns, first, (*field).clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let mut diagnostics = ChartDiagnostics {
            input_rows: input_rows as u64,
            ..ChartDiagnostics::default()
        };

        let compiled = match config {
            ChartConfig::Line {
                title,
                presentation,
                x_axis_mode,
                measures,
                split_by,
                order,
                aggregation,
                sampling,
                ..
            } => {
                let fields = BoundFields::new(&resolved, measures.len(), split_by.is_some());
                let candidates = extract_candidates(
                    source,
                    fields,
                    &measures,
                    split_by.as_ref(),
                    x_axis_mode,
                    &mut diagnostics,
                    budgets,
                )?;
                line::compile(
                    source,
                    title,
                    presentation,
                    &fields.x.reference,
                    x_axis_mode,
                    measures,
                    fields,
                    order,
                    aggregation,
                    sampling,
                    candidates,
                    config_hash,
                    diagnostics,
                    budgets,
                )?
            }
            ChartConfig::Bar {
                title,
                presentation,
                measures,
                split_by,
                order,
                aggregation,
                ..
            } => {
                let fields = BoundFields::new(&resolved, measures.len(), split_by.is_some());
                let candidates = extract_candidates(
                    source,
                    fields,
                    &measures,
                    split_by.as_ref(),
                    AxisMode::Category,
                    &mut diagnostics,
                    budgets,
                )?;
                bar::compile(
                    source,
                    title,
                    presentation,
                    measures,
                    fields,
                    order,
                    aggregation,
                    candidates,
                    config_hash,
                    diagnostics,
                    budgets,
                )?
            }
            ChartConfig::Scatter {
                title,
                presentation,
                measures,
                split_by,
                ..
            } => {
                let fields = BoundFields::new(&resolved, measures.len(), split_by.is_some());
                let candidates = extract_candidates(
                    source,
                    fields,
                    &measures,
                    split_by.as_ref(),
                    AxisMode::Value,
                    &mut diagnostics,
                    budgets,
                )?;
                scatter::compile(
                    source,
                    title,
                    presentation,
                    measures,
                    fields,
                    candidates,
                    config_hash,
                    diagnostics,
                    budgets,
                )?
            }
            ChartConfig::Heatmap {
                title,
                presentation,
                x,
                y,
                value,
                aggregation,
                ..
            } => heatmap::compile(
                source,
                title,
                presentation,
                x,
                y,
                value,
                aggregation,
                &resolved,
                config_hash,
                diagnostics,
                budgets,
            )?,
            ChartConfig::Boxplot {
                title,
                presentation,
                category,
                value,
                split_by,
                show_outliers,
                ..
            } => boxplot::compile(
                source,
                title,
                presentation,
                category,
                value,
                split_by,
                show_outliers,
                &resolved,
                config_hash,
                diagnostics,
                budgets,
            )?,
            ChartConfig::Histogram {
                title,
                presentation,
                value,
                bins,
                ..
            } => histogram::compile(
                source,
                title,
                presentation,
                value,
                bins,
                &resolved[0],
                config_hash,
                diagnostics,
                budgets,
            )?,
        };
        let response_bytes = serde_json::to_vec(&compiled.model)
            .map_err(|error| {
                ChartError::new(
                    "chart_materialization_failed",
                    format!("图表模型无法序列化: {error}"),
                )
            })?
            .len();
        enforce_budget("responseBytes", response_bytes, budgets.max_response_bytes)?;
        Ok(compiled)
    }
}

fn referenced_fields(config: &ChartConfig) -> Vec<&ChartFieldRef> {
    match config {
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
    }
}

#[derive(Clone, Copy)]
struct BoundFields<'a> {
    x: &'a ResolvedField,
    measures: &'a [ResolvedField],
    split: Option<&'a ResolvedField>,
}

impl<'a> BoundFields<'a> {
    fn new(resolved: &'a [ResolvedField], measure_count: usize, has_split: bool) -> Self {
        let split = has_split.then(|| &resolved[measure_count + 1]);
        Self {
            x: &resolved[0],
            measures: &resolved[1..=measure_count],
            split,
        }
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    measure_index: usize,
    split: Option<CategoryValue>,
    x: LineXValue,
    y: f64,
    ordinal: u64,
}

#[derive(Debug, Clone)]
struct HeatCandidate {
    x: CategoryValue,
    y: CategoryValue,
    value: f64,
}

#[derive(Debug, Clone)]
struct BoxCandidate {
    category: CategoryValue,
    split: Option<CategoryValue>,
    value: f64,
    ordinal: u64,
}

fn extract_candidates(
    source: &ChartSource,
    fields: BoundFields<'_>,
    measures: &[ChartFieldRef],
    split_by: Option<&ChartFieldRef>,
    x_axis_mode: AxisMode,
    diagnostics: &mut ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<Vec<Candidate>, ChartError> {
    enforce_budget("measures", measures.len(), budgets.max_measures)?;
    if let Some(split) = split_by
        && split.path.is_some()
    {
        return Err(incompatible("splitBy 只能引用标量类别字段"));
    }
    let candidate_count = source
        .batches
        .iter()
        .map(RecordBatch::num_rows)
        .sum::<usize>()
        .checked_mul(measures.len())
        .ok_or_else(|| too_large("rawValues", usize::MAX, budgets.max_raw_values))?;
    enforce_budget("rawValues", candidate_count, budgets.max_raw_values)?;
    diagnostics.candidate_points = candidate_count as u64;
    let mut candidates = Vec::with_capacity(candidate_count.min(budgets.max_points));
    let mut ordinal = 0u64;
    for batch in &source.batches {
        for row in 0..batch.num_rows() {
            for (measure_index, measure) in fields.measures.iter().enumerate() {
                let values = [
                    Some(fields.x.read(batch, row)?),
                    fields
                        .split
                        .map(|field| field.read(batch, row))
                        .transpose()?,
                    Some(measure.read(batch, row)?),
                ];
                diagnostics.missing_components += values
                    .iter()
                    .flatten()
                    .filter(|value| matches!(value, super::FieldValue::MissingComponent))
                    .count() as u64;
                if values.iter().flatten().any(|value| {
                    matches!(
                        value,
                        super::FieldValue::Missing | super::FieldValue::MissingComponent
                    )
                }) {
                    diagnostics.dropped_missing += 1;
                    continue;
                }
                if values
                    .iter()
                    .flatten()
                    .any(|value| matches!(value, super::FieldValue::NonFinite))
                {
                    diagnostics.dropped_non_finite += 1;
                    continue;
                }
                let x_value = values[0].as_ref().expect("x value is present");
                let x = match x_axis_mode {
                    AxisMode::Category => LineXValue::Category(x_value.as_category()?),
                    AxisMode::Value => LineXValue::Number(x_value.as_number()?),
                };
                let split = values[1]
                    .as_ref()
                    .map(|value| value.as_category())
                    .transpose()?;
                if matches!(split, Some(CategoryValue::Number { .. })) {
                    return Err(incompatible("splitBy 只允许字符串、布尔或整数类别字段"));
                }
                let y = values[2]
                    .as_ref()
                    .expect("measure value is present")
                    .as_number()?;
                candidates.push(Candidate {
                    measure_index,
                    split,
                    x,
                    y,
                    ordinal,
                });
            }
            ordinal += 1;
        }
    }
    Ok(candidates)
}

fn has_duplicate_keys(candidates: &[Candidate]) -> bool {
    let mut keys = HashSet::new();
    candidates.iter().any(|candidate| {
        !keys.insert((
            candidate.measure_index,
            candidate
                .split
                .as_ref()
                .map(category_key)
                .unwrap_or_default(),
            x_key(&candidate.x),
        ))
    })
}

fn candidate_values_are_valid(
    values: &[super::FieldValue],
    diagnostics: &mut ChartDiagnostics,
) -> bool {
    diagnostics.missing_components += values
        .iter()
        .filter(|value| matches!(value, super::FieldValue::MissingComponent))
        .count() as u64;
    if values.iter().any(|value| {
        matches!(
            value,
            super::FieldValue::Missing | super::FieldValue::MissingComponent
        )
    }) {
        diagnostics.dropped_missing += 1;
        return false;
    }
    if values
        .iter()
        .any(|value| matches!(value, super::FieldValue::NonFinite))
    {
        diagnostics.dropped_non_finite += 1;
        return false;
    }
    true
}

fn distinct_categories<'a>(values: impl Iterator<Item = &'a CategoryValue>) -> Vec<CategoryValue> {
    let mut seen = HashSet::new();
    values
        .filter(|value| seen.insert(category_key(value)))
        .cloned()
        .collect()
}

fn category_indices(categories: &[CategoryValue]) -> HashMap<String, usize> {
    categories
        .iter()
        .enumerate()
        .map(|(index, category)| (category_key(category), index))
        .collect()
}

fn box_series_keys(candidates: &[BoxCandidate]) -> Vec<Option<String>> {
    let mut ordered = candidates.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|candidate| candidate.ordinal);
    let mut seen = HashSet::new();
    ordered
        .into_iter()
        .map(|candidate| candidate.split.as_ref().map(category_key))
        .filter(|key| seen.insert(key.clone()))
        .collect()
}

fn box_series_names(
    keys: &[Option<String>],
    value: &ChartFieldRef,
) -> Result<Vec<String>, ChartError> {
    let names = keys
        .iter()
        .map(|key| {
            key.as_ref().map_or_else(
                || value.display_name(),
                |key| {
                    let category =
                        serde_json::from_str::<CategoryValue>(key).expect("canonical category key");
                    format!("{} · {}", value.display_name(), category_display(&category))
                },
            )
        })
        .collect::<Vec<_>>();
    let mut unique = HashSet::new();
    if names.iter().any(|name| !unique.insert(name.clone())) {
        return Err(ChartError::new(
            "chart_series_name_conflict",
            "图表系列名称冲突，请调整 label",
        ));
    }
    Ok(names)
}

fn box_summary(values: &[f64]) -> ([f64; 5], Vec<f64>) {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let q1 = quantile_r7(&sorted, 0.25);
    let median = quantile_r7(&sorted, 0.5);
    let q3 = quantile_r7(&sorted, 0.75);
    let fence = 1.5 * (q3 - q1);
    let lower = q1 - fence;
    let upper = q3 + fence;
    let whisker_min = sorted
        .iter()
        .copied()
        .find(|value| *value >= lower)
        .expect("q1 is inside the lower fence");
    let whisker_max = sorted
        .iter()
        .copied()
        .rev()
        .find(|value| *value <= upper)
        .expect("q3 is inside the upper fence");
    let outliers = sorted
        .into_iter()
        .filter(|value| *value < whisker_min || *value > whisker_max)
        .collect();
    ([whisker_min, q1, median, q3, whisker_max], outliers)
}

fn quantile_r7(sorted: &[f64], probability: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0];
    }
    let position = (sorted.len() - 1) as f64 * probability;
    let lower = position.floor() as usize;
    let fraction = position - lower as f64;
    sorted[lower] + fraction * (sorted[(lower + 1).min(sorted.len() - 1)] - sorted[lower])
}

fn compiled_from_model(model: ChartModel, diagnostics: ChartDiagnostics) -> CompiledChart {
    match &model {
        ChartModel::Line { series, .. } => CompiledChart {
            plot_data: PlotData::Line(
                series
                    .iter()
                    .flat_map(|series| {
                        series.points.iter().map(|point| PlotLineRow {
                            ordinal: point.ordinal,
                            x: point.x.clone(),
                            series: series.name.clone(),
                            value: point.y,
                        })
                    })
                    .collect(),
            ),
            plot_schema: line_plot_schema(),
            model,
            diagnostics,
        },
        ChartModel::Bar {
            categories, series, ..
        } => CompiledChart {
            plot_data: PlotData::Bar(
                series
                    .iter()
                    .flat_map(|series| {
                        categories
                            .iter()
                            .enumerate()
                            .map(|(category_index, category)| PlotBarRow {
                                category_index,
                                category: category.clone(),
                                series: series.name.clone(),
                                value: series.values[category_index],
                            })
                    })
                    .collect(),
            ),
            plot_schema: bar_plot_schema(),
            model,
            diagnostics,
        },
        ChartModel::Scatter { series, .. } => CompiledChart {
            plot_data: PlotData::Scatter(
                series
                    .iter()
                    .flat_map(|series| {
                        series.points.iter().map(|point| PlotScatterRow {
                            ordinal: point.ordinal,
                            series: series.name.clone(),
                            x: point.x,
                            y: point.y,
                        })
                    })
                    .collect(),
            ),
            plot_schema: scatter_plot_schema(),
            model,
            diagnostics,
        },
        ChartModel::Heatmap {
            x_categories,
            y_categories,
            cells,
            ..
        } => CompiledChart {
            plot_data: PlotData::Heatmap(
                cells
                    .iter()
                    .map(|cell| PlotHeatmapRow {
                        x_index: cell.x_index,
                        x_category: x_categories[cell.x_index].clone(),
                        y_index: cell.y_index,
                        y_category: y_categories[cell.y_index].clone(),
                        value: cell.value,
                    })
                    .collect(),
            ),
            plot_schema: heatmap_plot_schema(),
            model,
            diagnostics,
        },
        ChartModel::Boxplot {
            category_axis,
            series,
            ..
        } => CompiledChart {
            plot_data: PlotData::Boxplot(
                series
                    .iter()
                    .flat_map(|series| {
                        category_axis.categories.iter().enumerate().filter_map(
                            |(category_index, category)| {
                                series.values[category_index].map(|values| PlotBoxplotRow {
                                    category_index,
                                    category: category.clone(),
                                    series: series.name.clone(),
                                    values,
                                    outliers: series
                                        .outliers
                                        .iter()
                                        .filter(|outlier| outlier.category_index == category_index)
                                        .map(|outlier| outlier.value)
                                        .collect(),
                                })
                            },
                        )
                    })
                    .collect(),
            ),
            plot_schema: boxplot_plot_schema(),
            model,
            diagnostics,
        },
        ChartModel::Histogram { bins, .. } => CompiledChart {
            plot_data: PlotData::Histogram(
                bins.iter()
                    .enumerate()
                    .map(|(bin_index, bin)| PlotHistogramRow {
                        bin_index,
                        start: bin.start,
                        end: bin.end,
                        count: bin.count,
                        includes_end: bin.includes_end,
                    })
                    .collect(),
            ),
            plot_schema: histogram_plot_schema(),
            model,
            diagnostics,
        },
    }
}

fn ensure_output(diagnostics: &ChartDiagnostics) -> Result<(), ChartError> {
    if diagnostics.output_points == 0 {
        Err(no_data())
    } else {
        Ok(())
    }
}

fn no_data() -> ChartError {
    ChartError::new("chart_no_data", "图表配置没有产生可显示的数据")
}

fn incompatible(message: impl Into<String>) -> ChartError {
    ChartError::new("chart_column_incompatible", message)
}

fn enforce_budget(kind: &str, actual: usize, limit: usize) -> Result<(), ChartError> {
    if actual > limit {
        Err(too_large(kind, actual, limit))
    } else {
        Ok(())
    }
}

fn too_large(kind: &str, actual: usize, limit: usize) -> ChartError {
    ChartError::new("chart_too_large", "图表数据超过受控上限")
        .with_details(serde_json::json!({"kind": kind, "actual": actual, "limit": limit}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::builder::{Float64Builder, LargeListBuilder};
    use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, StringViewArray};
    use serde_json::{Value, json};

    use super::*;
    use crate::application::charting::{ChartConfig, ChartModel};
    use crate::application::dataframe_reader::DataFrameColumn;

    fn presentation() -> Value {
        json!({
            "legend":{"show":true,"position":"auto","orientation":"auto"},
            "palette":"colorblind","title":{"show":true},"axis":{"showGrid":true}
        })
    }

    fn line_presentation() -> Value {
        let mut value = presentation();
        value["line"] = json!({"width":2,"style":"solid","showSymbol":false});
        value
    }

    fn config(value: Value) -> ChartConfig {
        serde_json::from_value(value).unwrap()
    }

    fn column(name: &str, dtype: &str, unit: Option<&str>) -> DataFrameColumn {
        DataFrameColumn {
            name: name.into(),
            dtype: dtype.into(),
            nullable: true,
            unit: unit.map(str::to_owned),
        }
    }

    fn make_source(columns: Vec<DataFrameColumn>, arrays: Vec<(&str, ArrayRef)>) -> ChartSource {
        ChartSource::new(
            "7",
            columns,
            vec![RecordBatch::try_from_iter(arrays).unwrap()],
        )
    }

    #[test]
    fn empty_batches_return_no_data_instead_of_panicking() {
        let source = ChartSource::new(
            "7",
            vec![column("bus", "Utf8", None), column("load", "Float64", None)],
            vec![],
        );
        let chart_config = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),
            "x":{"column":"bus"},"xAxisMode":"category",
            "measures":[{"column":"load"}],"order":"source",
            "aggregation":"none","sampling":{"mode":"none"}
        }));

        assert_eq!(
            ChartCompiler::compile(&chart_config, &source)
                .unwrap_err()
                .code,
            "chart_no_data"
        );
    }

    #[test]
    fn three_phase_lines_expand_components_and_preserve_source_order() {
        let mut phases = LargeListBuilder::new(Float64Builder::new());
        phases.values().append_slice(&[1.01, 1.02, 1.03]);
        phases.append(true);
        phases.values().append_slice(&[0.98, 0.99, 1.00]);
        phases.append(true);
        let source = make_source(
            vec![
                column("bus", "Utf8View", None),
                column("v_mag", "LargeList(Float64)", Some("pu")),
            ],
            vec![
                ("bus", Arc::new(StringViewArray::from(vec!["b2", "b1"]))),
                ("v_mag", Arc::new(phases.finish())),
            ],
        );
        let chart_config = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),
            "x":{"column":"bus"},"xAxisMode":"category",
            "measures":[
                {"column":"v_mag","path":[0],"label":"A"},
                {"column":"v_mag","path":[1],"label":"B"},
                {"column":"v_mag","path":[2],"label":"C"}
            ],
            "order":"source","aggregation":"none","sampling":{"mode":"none"}
        }));

        let compiled = ChartCompiler::compile(&chart_config, &source).unwrap();
        let ChartModel::Line {
            series,
            y_axis,
            order,
            aggregation,
            ..
        } = &compiled.model
        else {
            panic!("expected line model")
        };
        assert_eq!(*order, ChartOrder::Source);
        assert_eq!(*aggregation, Aggregation::None);
        assert_eq!(
            series
                .iter()
                .map(|series| series.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B", "C"]
        );
        assert_eq!(
            series[0]
                .points
                .iter()
                .map(|point| point.y)
                .collect::<Vec<_>>(),
            [1.01, 0.98]
        );
        assert_eq!(
            series[0]
                .points
                .iter()
                .map(|point| point.ordinal)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(y_axis.unit.as_deref(), Some("pu"));
        assert_eq!(compiled.diagnostics.input_rows, 2);
        assert_eq!(compiled.diagnostics.candidate_points, 6);
        assert_eq!(compiled.diagnostics.output_points, 6);
    }

    #[test]
    fn value_line_aggregates_sorts_and_samples_only_after_aggregation() {
        let source = make_source(
            vec![
                column("x", "Float64", None),
                column("y", "Float64", Some("kV")),
            ],
            vec![
                ("x", Arc::new(Float64Array::from(vec![2.0, 1.0, 1.0, 3.0]))),
                (
                    "y",
                    Arc::new(Float64Array::from(vec![20.0, 10.0, 30.0, 40.0])),
                ),
            ],
        );
        let aggregate = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"x"},
            "xAxisMode":"value","measures":[{"column":"y"}],"order":"xAsc","aggregation":"mean",
            "sampling":{"mode":"none"}
        }));
        let compiled = ChartCompiler::compile(&aggregate, &source).unwrap();
        let ChartModel::Line { series, .. } = &compiled.model else {
            panic!()
        };
        assert_eq!(
            series[0]
                .points
                .iter()
                .map(|point| point.y)
                .collect::<Vec<_>>(),
            [20.0, 20.0, 40.0]
        );
        assert_eq!(compiled.diagnostics.aggregated_groups, 1);

        let x = (0..101).map(|value| value as f64).collect::<Vec<_>>();
        let y = x.iter().map(|value| value.sin()).collect::<Vec<_>>();
        let source = make_source(
            vec![column("x", "Float64", None), column("y", "Float64", None)],
            vec![
                ("x", Arc::new(Float64Array::from(x))),
                ("y", Arc::new(Float64Array::from(y))),
            ],
        );
        let sampled = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"x"},
            "xAxisMode":"value","measures":[{"column":"y"}],"order":"xAsc","aggregation":"none",
            "sampling":{"mode":"lttb","maxPointsPerSeries":100}
        }));
        let compiled = ChartCompiler::compile(&sampled, &source).unwrap();
        let ChartModel::Line { series, .. } = &compiled.model else {
            panic!()
        };
        assert_eq!(series[0].points.len(), 100);
        assert_eq!(series[0].points.first().unwrap().ordinal, 0);
        assert_eq!(series[0].points.last().unwrap().ordinal, 100);
        assert_eq!(compiled.diagnostics.sampled_points, 1);

        let exact_threshold_source = make_source(
            vec![column("x", "Float64", None), column("y", "Float64", None)],
            vec![
                (
                    "x",
                    Arc::new(Float64Array::from_iter_values(
                        (0..100).map(|value| value as f64),
                    )),
                ),
                (
                    "y",
                    Arc::new(Float64Array::from_iter_values(
                        (0..100).map(|value| value as f64),
                    )),
                ),
            ],
        );
        assert_eq!(
            ChartCompiler::compile(&sampled, &exact_threshold_source)
                .unwrap_err()
                .code,
            "chart_config_invalid"
        );
    }

    #[test]
    fn reducers_source_duplicates_and_descending_ties_are_frozen() {
        let source = make_source(
            vec![column("x", "Float64", None), column("y", "Float64", None)],
            vec![
                ("x", Arc::new(Float64Array::from(vec![2.0, 1.0, 1.0, 3.0]))),
                (
                    "y",
                    Arc::new(Float64Array::from(vec![20.0, 10.0, 30.0, 40.0])),
                ),
            ],
        );
        for (aggregation, expected) in [("mean", 20.0), ("min", 10.0), ("max", 30.0), ("sum", 40.0)]
        {
            let chart_config = config(json!({
                "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"x"},
                "xAxisMode":"value","measures":[{"column":"y"}],"order":"xAsc",
                "aggregation":aggregation,"sampling":{"mode":"none"}
            }));
            let compiled = ChartCompiler::compile(&chart_config, &source).unwrap();
            let ChartModel::Line { series, .. } = compiled.model else {
                panic!()
            };
            assert_eq!(series[0].points[0].y, expected);
        }

        let duplicates = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"x"},
            "xAxisMode":"value","measures":[{"column":"y"}],"order":"source",
            "aggregation":"none","sampling":{"mode":"none"}
        }));
        let compiled = ChartCompiler::compile(&duplicates, &source).unwrap();
        let ChartModel::Line { series, .. } = compiled.model else {
            panic!()
        };
        assert_eq!(
            series[0]
                .points
                .iter()
                .map(|point| (point.x.clone(), point.ordinal))
                .collect::<Vec<_>>(),
            [
                (LineXValue::Number(2.0), 0),
                (LineXValue::Number(1.0), 1),
                (LineXValue::Number(1.0), 2),
                (LineXValue::Number(3.0), 3),
            ]
        );

        let descending = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"x"},
            "xAxisMode":"value","measures":[{"column":"y"}],"order":"xDesc",
            "aggregation":"none","sampling":{"mode":"none"}
        }));
        let compiled = ChartCompiler::compile(&descending, &source).unwrap();
        let ChartModel::Line { series, .. } = compiled.model else {
            panic!()
        };
        assert_eq!(
            series[0]
                .points
                .iter()
                .map(|point| point.ordinal)
                .collect::<Vec<_>>(),
            [3, 0, 1, 2]
        );
    }

    #[test]
    fn split_series_are_measure_major_and_typed_categories_do_not_collide() {
        let source = make_source(
            vec![
                column("bus", "Utf8View", None),
                column("phase", "Utf8View", None),
                column("p", "Float64", Some("MW")),
                column("q", "Float64", Some("MW")),
            ],
            vec![
                ("bus", Arc::new(StringViewArray::from(vec!["b1", "b2"]))),
                ("phase", Arc::new(StringViewArray::from(vec!["B", "A"]))),
                ("p", Arc::new(Float64Array::from(vec![1.0, 2.0]))),
                ("q", Arc::new(Float64Array::from(vec![3.0, 4.0]))),
            ],
        );
        let chart_config = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},
            "xAxisMode":"category","measures":[{"column":"p"},{"column":"q"}],
            "splitBy":{"column":"phase"},"order":"source","aggregation":"none","sampling":{"mode":"none"}
        }));
        let compiled = ChartCompiler::compile(&chart_config, &source).unwrap();
        let ChartModel::Line { series, .. } = &compiled.model else {
            panic!()
        };
        assert_eq!(
            series
                .iter()
                .map(|series| series.name.as_str())
                .collect::<Vec<_>>(),
            ["p · B", "p · A", "q · B", "q · A"]
        );
        assert_ne!(
            category_key(&CategoryValue::String { value: "1".into() }),
            category_key(&CategoryValue::Integer { value: "1".into() })
        );
        assert_ne!(
            category_key(&CategoryValue::Integer { value: "1".into() }),
            category_key(&CategoryValue::Number { value: 1.0 })
        );

        let collision = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},
            "xAxisMode":"category","measures":[
                {"column":"p","label":"same"},{"column":"q","label":"same"}
            ],"order":"source","aggregation":"none","sampling":{"mode":"none"}
        }));
        assert_eq!(
            ChartCompiler::compile(&collision, &source)
                .unwrap_err()
                .code,
            "chart_series_name_conflict"
        );
    }

    #[test]
    fn bar_requires_explicit_aggregation_and_scatter_preserves_duplicates() {
        let source = make_source(
            vec![
                column("bus", "Utf8", None),
                column("value", "Float64", None),
            ],
            vec![
                ("bus", Arc::new(StringArray::from(vec!["b1", "b1", "b2"]))),
                ("value", Arc::new(Float64Array::from(vec![1.0, 3.0, 2.0]))),
            ],
        );
        let bar = config(json!({
            "version":1,"kind":"bar","presentation":presentation(),"x":{"column":"bus"},
            "measures":[{"column":"value"}],"order":"source","aggregation":"none"
        }));
        assert_eq!(
            ChartCompiler::compile(&bar, &source).unwrap_err().code,
            "chart_duplicate_requires_aggregation"
        );
        let bar = config(json!({
            "version":1,"kind":"bar","presentation":presentation(),"x":{"column":"bus"},
            "measures":[{"column":"value"}],"order":"source","aggregation":"mean"
        }));
        let compiled = ChartCompiler::compile(&bar, &source).unwrap();
        let ChartModel::Bar {
            categories, series, ..
        } = &compiled.model
        else {
            panic!()
        };
        assert_eq!(categories.len(), 2);
        assert_eq!(series[0].values, vec![Some(2.0), Some(2.0)]);

        let source = make_source(
            vec![column("x", "Float64", None), column("y", "Float64", None)],
            vec![
                ("x", Arc::new(Float64Array::from(vec![1.0, 1.0]))),
                ("y", Arc::new(Float64Array::from(vec![2.0, 2.0]))),
            ],
        );
        let scatter = config(json!({
            "version":1,"kind":"scatter","presentation":presentation(),"x":{"column":"x"},
            "measures":[{"column":"y"}]
        }));
        let compiled = ChartCompiler::compile(&scatter, &source).unwrap();
        let ChartModel::Scatter { series, .. } = &compiled.model else {
            panic!()
        };
        assert_eq!(series[0].points.len(), 2);
        assert_eq!(series[0].points[0].ordinal, 0);
        assert_eq!(series[0].points[1].ordinal, 1);
    }

    #[test]
    fn diagnostics_distinguish_missing_components_non_finite_and_empty_output() {
        let mut values = LargeListBuilder::new(Float64Builder::new());
        values.values().append_slice(&[1.0, 2.0, 3.0]);
        values.append(true);
        values.values().append_slice(&[4.0, 5.0]);
        values.append(true);
        values.values().append_slice(&[f64::NAN, 7.0, 8.0]);
        values.append(true);
        values.append(false);
        let source = make_source(
            vec![
                column("bus", "Utf8View", None),
                column("v", "LargeList(Float64)", None),
            ],
            vec![
                (
                    "bus",
                    Arc::new(StringViewArray::from(vec!["a", "b", "c", "d"])),
                ),
                ("v", Arc::new(values.finish())),
            ],
        );
        let chart_config = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},
            "xAxisMode":"category","measures":[{"column":"v","path":[2]}],"order":"source",
            "aggregation":"none","sampling":{"mode":"none"}
        }));
        let compiled = ChartCompiler::compile(&chart_config, &source).unwrap();
        assert_eq!(compiled.diagnostics.candidate_points, 4);
        assert_eq!(compiled.diagnostics.missing_components, 1);
        assert_eq!(compiled.diagnostics.dropped_missing, 2);
        assert_eq!(compiled.diagnostics.dropped_non_finite, 0);
        assert_eq!(compiled.diagnostics.output_points, 2);

        let missing = config(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},
            "xAxisMode":"category","measures":[{"column":"v","path":[9]}],"order":"source",
            "aggregation":"none","sampling":{"mode":"none"}
        }));
        assert_eq!(
            ChartCompiler::compile(&missing, &source).unwrap_err().code,
            "chart_no_data"
        );

        let non_finite_source = make_source(
            vec![column("x", "Float64", None), column("y", "Float64", None)],
            vec![
                ("x", Arc::new(Float64Array::from(vec![0.0, 1.0]))),
                ("y", Arc::new(Float64Array::from(vec![2.0, f64::NAN]))),
            ],
        );
        let non_finite = config(json!({
            "version":1,"kind":"scatter","presentation":presentation(),"x":{"column":"x"},
            "measures":[{"column":"y"}]
        }));
        let compiled = ChartCompiler::compile(&non_finite, &non_finite_source).unwrap();
        assert_eq!(compiled.diagnostics.dropped_non_finite, 1);
        assert_eq!(compiled.diagnostics.output_points, 1);
    }

    #[test]
    fn unit_series_and_point_budgets_fail_closed() {
        let source = make_source(
            vec![
                column("x", "Float64", None),
                column("a", "Float64", Some("MW")),
                column("b", "Float64", Some("Mvar")),
            ],
            vec![
                ("x", Arc::new(Float64Array::from(vec![1.0]))),
                ("a", Arc::new(Float64Array::from(vec![2.0]))),
                ("b", Arc::new(Float64Array::from(vec![3.0]))),
            ],
        );
        let conflict = config(json!({
            "version":1,"kind":"scatter","presentation":presentation(),"x":{"column":"x"},
            "measures":[{"column":"a"},{"column":"b"}]
        }));
        assert_eq!(
            ChartCompiler::compile(&conflict, &source).unwrap_err().code,
            "chart_unit_conflict"
        );

        let categories = (0..101)
            .map(|index| format!("s{index}"))
            .collect::<Vec<_>>();
        let source = make_source(
            vec![
                column("x", "Float64", None),
                column("split", "Utf8", None),
                column("y", "Float64", None),
            ],
            vec![
                (
                    "x",
                    Arc::new(Float64Array::from_iter_values(
                        (0..101).map(|value| value as f64),
                    )) as ArrayRef,
                ),
                (
                    "split",
                    Arc::new(StringArray::from_iter_values(categories.iter())) as ArrayRef,
                ),
                (
                    "y",
                    Arc::new(Float64Array::from_iter_values(
                        (0..101).map(|value| value as f64),
                    )) as ArrayRef,
                ),
            ],
        );
        let too_many_series = config(json!({
            "version":1,"kind":"scatter","presentation":presentation(),"x":{"column":"x"},
            "measures":[{"column":"y"}],"splitBy":{"column":"split"}
        }));
        assert_eq!(
            ChartCompiler::compile(&too_many_series, &source)
                .unwrap_err()
                .code,
            "chart_too_large"
        );

        let count = 50_001;
        let source = make_source(
            vec![column("x", "Float64", None), column("y", "Float64", None)],
            vec![
                (
                    "x",
                    Arc::new(Float64Array::from_iter_values(
                        (0..count).map(|value| value as f64),
                    )),
                ),
                (
                    "y",
                    Arc::new(Float64Array::from_iter_values(
                        (0..count).map(|value| value as f64),
                    )),
                ),
            ],
        );
        let too_many_points = config(json!({
            "version":1,"kind":"scatter","presentation":presentation(),"x":{"column":"x"},
            "measures":[{"column":"y"}]
        }));
        assert_eq!(
            ChartCompiler::compile(&too_many_points, &source)
                .unwrap_err()
                .code,
            "chart_too_large"
        );
    }

    #[test]
    fn heatmap_reducers_and_count_use_only_complete_finite_cells() {
        let source = make_source(
            vec![
                column("x", "Utf8", None),
                column("y", "Utf8", None),
                column("value", "Float64", Some("MW")),
            ],
            vec![
                ("x", Arc::new(StringArray::from(vec!["a", "a", "b"]))),
                ("y", Arc::new(StringArray::from(vec!["r", "r", "s"]))),
                ("value", Arc::new(Float64Array::from(vec![1.0, 3.0, 5.0]))),
            ],
        );
        for (aggregation, expected) in [
            ("mean", 2.0),
            ("min", 1.0),
            ("max", 3.0),
            ("sum", 4.0),
            ("count", 2.0),
        ] {
            let chart_config = config(json!({
                "version":1,"kind":"heatmap","presentation":presentation(),
                "x":{"column":"x"},"y":{"column":"y"},"value":{"column":"value"},
                "aggregation":aggregation
            }));
            let compiled = ChartCompiler::compile(&chart_config, &source).unwrap();
            let ChartModel::Heatmap { cells, .. } = compiled.model else {
                panic!()
            };
            assert_eq!(cells.len(), 2);
            assert_eq!(cells[0].value, expected);
            assert_eq!(compiled.diagnostics.aggregated_groups, 1);
        }

        let incomplete = make_source(
            vec![
                column("x", "Utf8", None),
                column("y", "Utf8", None),
                column("value", "Float64", None),
            ],
            vec![
                (
                    "x",
                    Arc::new(StringArray::from(vec![Some("a"), Some("a"), None])),
                ),
                ("y", Arc::new(StringArray::from(vec!["r", "r", "r"]))),
                (
                    "value",
                    Arc::new(Float64Array::from(vec![Some(1.0), None, Some(3.0)])),
                ),
            ],
        );
        let count = config(json!({
            "version":1,"kind":"heatmap","presentation":presentation(),
            "x":{"column":"x"},"y":{"column":"y"},"value":{"column":"value"},
            "aggregation":"count"
        }));
        let compiled = ChartCompiler::compile(&count, &incomplete).unwrap();
        assert_eq!(compiled.diagnostics.dropped_missing, 2);
        assert_eq!(compiled.diagnostics.output_points, 1);
    }

    #[test]
    fn boxplot_uses_r7_quartiles_tukey_whiskers_and_optional_outliers() {
        let source = make_source(
            vec![
                column("category", "Utf8", None),
                column("value", "Float64", Some("kV")),
            ],
            vec![
                (
                    "category",
                    Arc::new(StringArray::from(vec!["A", "A", "A", "A", "A", "B"])),
                ),
                (
                    "value",
                    Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 100.0, 5.0])),
                ),
            ],
        );
        let chart_config = config(json!({
            "version":1,"kind":"boxplot","presentation":presentation(),
            "category":{"column":"category"},"value":{"column":"value"},
            "showOutliers":true
        }));
        let compiled = ChartCompiler::compile(&chart_config, &source).unwrap();
        let ChartModel::Boxplot { series, .. } = compiled.model else {
            panic!()
        };
        assert_eq!(series[0].values[0], Some([1.0, 2.0, 3.0, 4.0, 4.0]));
        assert_eq!(series[0].values[1], Some([5.0; 5]));
        assert_eq!(series[0].outliers.len(), 1);
        assert_eq!(series[0].outliers[0].value, 100.0);

        let hidden = config(json!({
            "version":1,"kind":"boxplot","presentation":presentation(),
            "category":{"column":"category"},"value":{"column":"value"},
            "showOutliers":false
        }));
        let compiled = ChartCompiler::compile(&hidden, &source).unwrap();
        let ChartModel::Boxplot { series, .. } = compiled.model else {
            panic!()
        };
        assert!(series[0].outliers.is_empty());
    }

    #[test]
    fn histogram_freezes_equal_and_normal_bin_boundaries_and_rejects_empty_data() {
        let equal = make_source(
            vec![column("value", "Float64", Some("Hz"))],
            vec![("value", Arc::new(Float64Array::from(vec![2.0, 2.0, 2.0])))],
        );
        let chart_config = config(json!({
            "version":1,"kind":"histogram","presentation":presentation(),
            "value":{"column":"value"},"bins":5
        }));
        let compiled = ChartCompiler::compile(&chart_config, &equal).unwrap();
        let ChartModel::Histogram { bins, .. } = compiled.model else {
            panic!()
        };
        assert_eq!(bins.len(), 1);
        assert_eq!((bins[0].start, bins[0].end, bins[0].count), (2.0, 2.0, 3));
        assert!(bins[0].includes_end);

        let normal = make_source(
            vec![column("value", "Float64", None)],
            vec![(
                "value",
                Arc::new(Float64Array::from(vec![0.0, 1.0, 2.0, 3.0])),
            )],
        );
        let normal_config = config(json!({
            "version":1,"kind":"histogram","presentation":presentation(),
            "value":{"column":"value"},"bins":3
        }));
        let compiled = ChartCompiler::compile(&normal_config, &normal).unwrap();
        let ChartModel::Histogram { bins, .. } = compiled.model else {
            panic!()
        };
        assert_eq!(
            bins.iter().map(|bin| bin.count).collect::<Vec<_>>(),
            [1, 1, 2]
        );
        assert!(!bins[0].includes_end);
        assert!(bins[2].includes_end);

        let empty = make_source(
            vec![column("value", "Float64", None)],
            vec![(
                "value",
                Arc::new(Float64Array::from(vec![Some(f64::NAN), None])),
            )],
        );
        assert_eq!(
            ChartCompiler::compile(&normal_config, &empty)
                .unwrap_err()
                .code,
            "chart_no_data"
        );
    }

    #[test]
    fn remaining_chart_kinds_enforce_raw_category_and_cell_budgets() {
        let heatmap_source = make_source(
            vec![
                column("x", "Utf8", None),
                column("y", "Utf8", None),
                column("value", "Float64", None),
            ],
            vec![
                ("x", Arc::new(StringArray::from(vec!["a", "b"]))),
                ("y", Arc::new(StringArray::from(vec!["r", "s"]))),
                ("value", Arc::new(Float64Array::from(vec![1.0, 2.0]))),
            ],
        );
        let heatmap = config(json!({
            "version":1,"kind":"heatmap","presentation":presentation(),
            "x":{"column":"x"},"y":{"column":"y"},"value":{"column":"value"},
            "aggregation":"mean"
        }));
        let budgets = ChartBudgets {
            max_heatmap_cells: 1,
            ..ChartBudgets::default()
        };
        assert_eq!(
            ChartCompiler::compile_with_budgets(&heatmap, &heatmap_source, budgets)
                .unwrap_err()
                .code,
            "chart_too_large"
        );

        let boxplot = config(json!({
            "version":1,"kind":"boxplot","presentation":presentation(),
            "category":{"column":"x"},"value":{"column":"value"},"showOutliers":false
        }));
        let budgets = ChartBudgets {
            max_categories: 1,
            ..ChartBudgets::default()
        };
        assert_eq!(
            ChartCompiler::compile_with_budgets(&boxplot, &heatmap_source, budgets)
                .unwrap_err()
                .code,
            "chart_too_large"
        );

        let histogram = config(json!({
            "version":1,"kind":"histogram","presentation":presentation(),
            "value":{"column":"value"},"bins":2
        }));
        let budgets = ChartBudgets {
            max_raw_values: 1,
            ..ChartBudgets::default()
        };
        assert_eq!(
            ChartCompiler::compile_with_budgets(&histogram, &heatmap_source, budgets)
                .unwrap_err()
                .code,
            "chart_too_large"
        );
    }
}
