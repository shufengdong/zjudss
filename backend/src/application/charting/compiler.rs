use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use arrow_array::RecordBatch;

use super::contracts::normalize_chart_config;
use super::{
    Aggregation, AxisMode, BarSeries, CHART_CONTRACT_VERSION, CategoryAxis, CategoryAxisType,
    CategoryValue, ChartBudgets, ChartConfig, ChartDiagnostics, ChartError, ChartFieldRef,
    ChartModel, ChartModelBase, ChartOrder, ChartSampling, LinePoint, LineSeries, LineXValue,
    MixedAxis, NumericPoint, ResolvedField, ScatterSeries, ValueAxis,
};
use crate::application::dataframe_reader::DataFrameColumn;

#[derive(Debug, Clone)]
pub struct ChartSource {
    pub dataset_id: String,
    pub columns: Vec<DataFrameColumn>,
    pub batches: Vec<RecordBatch>,
}

impl ChartSource {
    pub fn new(
        dataset_id: impl Into<String>,
        columns: Vec<DataFrameColumn>,
        batches: Vec<RecordBatch>,
    ) -> Self {
        Self {
            dataset_id: dataset_id.into(),
            columns,
            batches,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PlotData {
    Line(Vec<PlotLineRow>),
    Bar(Vec<PlotBarRow>),
    Scatter(Vec<PlotScatterRow>),
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
                compile_line(
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
                compile_bar(
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
                compile_scatter(
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
            _ => {
                return Err(ChartError::new(
                    "chart_config_invalid",
                    "该图表类型尚未由标量编译器实现",
                ));
            }
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

#[allow(clippy::too_many_arguments)]
fn compile_line(
    source: &ChartSource,
    title: Option<String>,
    presentation: super::LineChartPresentation,
    x: &ChartFieldRef,
    x_axis_mode: AxisMode,
    measures: Vec<ChartFieldRef>,
    fields: BoundFields<'_>,
    order: ChartOrder,
    aggregation: Aggregation,
    sampling: ChartSampling,
    candidates: Vec<Candidate>,
    config_hash: String,
    mut diagnostics: ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<CompiledChart, ChartError> {
    let unit = common_unit(fields.measures)?;
    let mut candidates = aggregate_candidates(candidates, aggregation, &mut diagnostics)?;
    order_candidates(&mut candidates, order);
    if x_axis_mode == AxisMode::Category {
        let category_count = candidates
            .iter()
            .map(|candidate| x_key(&candidate.x))
            .collect::<HashSet<_>>()
            .len();
        enforce_budget("categories", category_count, budgets.max_categories)?;
    }
    let series_keys = series_keys(&candidates);
    enforce_budget("series", series_keys.len(), budgets.max_series)?;
    let names = series_names(&series_keys, &measures)?;
    let mut series = Vec::with_capacity(series_keys.len());
    for (series_index, key) in series_keys.iter().enumerate() {
        let mut points = candidates
            .iter()
            .filter(|candidate| series_key(candidate) == *key)
            .map(|candidate| LinePoint {
                x: candidate.x.clone(),
                y: candidate.y,
                ordinal: candidate.ordinal,
            })
            .collect::<Vec<_>>();
        if let ChartSampling::Lttb {
            max_points_per_series,
        } = sampling
        {
            if max_points_per_series >= points.len() {
                return Err(ChartError::new(
                    "chart_config_invalid",
                    "maxPointsPerSeries 必须小于每个系列的输入点数",
                ));
            }
            let numeric = points
                .iter()
                .map(|point| NumericPoint {
                    x: match point.x {
                        LineXValue::Number(value) => value,
                        LineXValue::Category(_) => {
                            unreachable!("normalization rejects category LTTB")
                        }
                    },
                    y: point.y,
                    ordinal: point.ordinal,
                })
                .collect::<Vec<_>>();
            let sampled = super::lttb(&numeric, max_points_per_series);
            diagnostics.sampled_points += (numeric.len() - sampled.len()) as u64;
            points = sampled
                .into_iter()
                .map(|point| LinePoint {
                    x: LineXValue::Number(point.x),
                    y: point.y,
                    ordinal: point.ordinal,
                })
                .collect();
        }
        series.push(LineSeries {
            name: names[series_index].clone(),
            points,
        });
    }
    diagnostics.output_points = series.iter().map(|item| item.points.len() as u64).sum();
    ensure_output(&diagnostics)?;
    enforce_budget(
        "points",
        diagnostics.output_points as usize,
        budgets.max_points,
    )?;
    let base = model_base(source, title, config_hash, diagnostics.clone());
    let model = ChartModel::Line {
        base,
        presentation,
        x_axis: MixedAxis {
            r#type: x_axis_mode,
            name: x.display_name(),
            unit: fields.x.unit.clone(),
        },
        y_axis: ValueAxis {
            name: (measures.len() == 1).then(|| measures[0].display_name()),
            unit,
        },
        series,
    };
    Ok(compiled_from_model(model, diagnostics))
}

#[allow(clippy::too_many_arguments)]
fn compile_bar(
    source: &ChartSource,
    title: Option<String>,
    presentation: super::ChartPresentation,
    measures: Vec<ChartFieldRef>,
    fields: BoundFields<'_>,
    order: ChartOrder,
    aggregation: Aggregation,
    candidates: Vec<Candidate>,
    config_hash: String,
    mut diagnostics: ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<CompiledChart, ChartError> {
    let unit = common_unit(fields.measures)?;
    if aggregation == Aggregation::None && has_duplicate_keys(&candidates) {
        return Err(ChartError::new(
            "chart_duplicate_requires_aggregation",
            "柱状图存在重复类别，必须显式选择聚合方式",
        ));
    }
    let candidates = aggregate_candidates(candidates, aggregation, &mut diagnostics)?;
    let series_keys = series_keys(&candidates);
    enforce_budget("series", series_keys.len(), budgets.max_series)?;
    let names = series_names(&series_keys, &measures)?;
    let mut categories = Vec::<CategoryValue>::new();
    let mut seen = HashSet::new();
    for candidate in &candidates {
        let LineXValue::Category(category) = &candidate.x else {
            unreachable!("bar uses category x")
        };
        if seen.insert(category_key(category)) {
            categories.push(category.clone());
        }
    }
    match order {
        ChartOrder::Source => {}
        ChartOrder::XAsc => categories.sort_by(compare_categories),
        ChartOrder::XDesc => categories.sort_by(|left, right| compare_categories(right, left)),
    }
    enforce_budget("categories", categories.len(), budgets.max_categories)?;
    let mut series = Vec::with_capacity(series_keys.len());
    for (series_index, key) in series_keys.iter().enumerate() {
        let by_category = candidates
            .iter()
            .filter(|candidate| series_key(candidate) == *key)
            .map(|candidate| {
                let LineXValue::Category(category) = &candidate.x else {
                    unreachable!()
                };
                (category_key(category), candidate.y)
            })
            .collect::<HashMap<_, _>>();
        series.push(BarSeries {
            name: names[series_index].clone(),
            values: categories
                .iter()
                .map(|category| by_category.get(&category_key(category)).copied())
                .collect(),
        });
    }
    diagnostics.output_points = series
        .iter()
        .flat_map(|item| item.values.iter())
        .filter(|value| value.is_some())
        .count() as u64;
    ensure_output(&diagnostics)?;
    enforce_budget(
        "points",
        diagnostics.output_points as usize,
        budgets.max_points,
    )?;
    let base = model_base(source, title, config_hash, diagnostics.clone());
    let model = ChartModel::Bar {
        base,
        presentation,
        x_axis: CategoryAxis {
            r#type: CategoryAxisType::Category,
            name: fields.x.reference.display_name(),
            unit: fields.x.unit.clone(),
        },
        y_axis: ValueAxis {
            name: (measures.len() == 1).then(|| measures[0].display_name()),
            unit,
        },
        categories,
        series,
    };
    Ok(compiled_from_model(model, diagnostics))
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

#[allow(clippy::too_many_arguments)]
fn compile_scatter(
    source: &ChartSource,
    title: Option<String>,
    presentation: super::ChartPresentation,
    measures: Vec<ChartFieldRef>,
    fields: BoundFields<'_>,
    candidates: Vec<Candidate>,
    config_hash: String,
    mut diagnostics: ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<CompiledChart, ChartError> {
    let unit = common_unit(fields.measures)?;
    let series_keys = series_keys(&candidates);
    enforce_budget("series", series_keys.len(), budgets.max_series)?;
    let names = series_names(&series_keys, &measures)?;
    let series = series_keys
        .iter()
        .enumerate()
        .map(|(series_index, key)| ScatterSeries {
            name: names[series_index].clone(),
            points: candidates
                .iter()
                .filter(|candidate| series_key(candidate) == *key)
                .map(|candidate| NumericPoint {
                    x: match candidate.x {
                        LineXValue::Number(value) => value,
                        LineXValue::Category(_) => unreachable!("scatter uses numeric x"),
                    },
                    y: candidate.y,
                    ordinal: candidate.ordinal,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    diagnostics.output_points = series.iter().map(|item| item.points.len() as u64).sum();
    ensure_output(&diagnostics)?;
    enforce_budget(
        "points",
        diagnostics.output_points as usize,
        budgets.max_points,
    )?;
    let base = model_base(source, title, config_hash, diagnostics.clone());
    let model = ChartModel::Scatter {
        base,
        presentation,
        x_axis: MixedAxis {
            r#type: AxisMode::Value,
            name: fields.x.reference.display_name(),
            unit: fields.x.unit.clone(),
        },
        y_axis: ValueAxis {
            name: (measures.len() == 1).then(|| measures[0].display_name()),
            unit,
        },
        series,
    };
    Ok(compiled_from_model(model, diagnostics))
}

fn aggregate_candidates(
    candidates: Vec<Candidate>,
    aggregation: Aggregation,
    diagnostics: &mut ChartDiagnostics,
) -> Result<Vec<Candidate>, ChartError> {
    let mut positions = HashMap::<(usize, String, String), usize>::new();
    let mut groups = Vec::<(Candidate, usize, f64, f64, f64)>::new();
    for candidate in candidates {
        let key = (
            candidate.measure_index,
            candidate
                .split
                .as_ref()
                .map(category_key)
                .unwrap_or_default(),
            x_key(&candidate.x),
        );
        if let Some(position) = positions.get(&key).copied() {
            if aggregation == Aggregation::None {
                groups.push((candidate, 1, 0.0, 0.0, 0.0));
                continue;
            }
            let group = &mut groups[position];
            group.1 += 1;
            group.2 += candidate.y;
            group.3 = group.3.min(candidate.y);
            group.4 = group.4.max(candidate.y);
        } else {
            let position = groups.len();
            positions.insert(key, position);
            let value = candidate.y;
            groups.push((candidate, 1, value, value, value));
        }
    }
    if aggregation == Aggregation::None {
        return Ok(groups.into_iter().map(|group| group.0).collect());
    }
    diagnostics.aggregated_groups += groups.iter().filter(|group| group.1 > 1).count() as u64;
    Ok(groups
        .into_iter()
        .map(|(mut candidate, count, sum, min, max)| {
            candidate.y = match aggregation {
                Aggregation::None => unreachable!(),
                Aggregation::Mean => sum / count as f64,
                Aggregation::Min => min,
                Aggregation::Max => max,
                Aggregation::Sum => sum,
            };
            candidate
        })
        .collect())
}

fn order_candidates(candidates: &mut [Candidate], order: ChartOrder) {
    match order {
        ChartOrder::Source => candidates.sort_by_key(|candidate| candidate.ordinal),
        ChartOrder::XAsc => candidates.sort_by(compare_candidate_x),
        ChartOrder::XDesc => candidates.sort_by(|left, right| {
            compare_x(&right.x, &left.x).then_with(|| left.ordinal.cmp(&right.ordinal))
        }),
    }
}

fn compare_candidate_x(left: &Candidate, right: &Candidate) -> Ordering {
    compare_x(&left.x, &right.x).then_with(|| left.ordinal.cmp(&right.ordinal))
}

fn compare_x(left: &LineXValue, right: &LineXValue) -> Ordering {
    match (left, right) {
        (LineXValue::Number(left), LineXValue::Number(right)) => left.total_cmp(right),
        (LineXValue::Category(left), LineXValue::Category(right)) => {
            compare_categories(left, right)
        }
        (LineXValue::Category(_), LineXValue::Number(_)) => Ordering::Less,
        (LineXValue::Number(_), LineXValue::Category(_)) => Ordering::Greater,
    }
}

fn compare_categories(left: &CategoryValue, right: &CategoryValue) -> Ordering {
    match (left, right) {
        (CategoryValue::String { value: left }, CategoryValue::String { value: right }) => {
            left.cmp(right)
        }
        (CategoryValue::Boolean { value: left }, CategoryValue::Boolean { value: right }) => {
            left.cmp(right)
        }
        (CategoryValue::Integer { value: left }, CategoryValue::Integer { value: right }) => left
            .parse::<i128>()
            .expect("validated integer")
            .cmp(&right.parse::<i128>().expect("validated integer")),
        (CategoryValue::Number { value: left }, CategoryValue::Number { value: right }) => {
            left.total_cmp(right)
        }
        _ => category_rank(left).cmp(&category_rank(right)),
    }
}

fn category_rank(value: &CategoryValue) -> u8 {
    match value {
        CategoryValue::String { .. } => 0,
        CategoryValue::Boolean { .. } => 1,
        CategoryValue::Integer { .. } => 2,
        CategoryValue::Number { .. } => 3,
    }
}

fn series_keys(candidates: &[Candidate]) -> Vec<(usize, Option<String>)> {
    let mut split_first = HashMap::<String, u64>::new();
    for candidate in candidates {
        if let Some(split) = &candidate.split {
            split_first
                .entry(category_key(split))
                .or_insert(candidate.ordinal);
        }
    }
    let mut keys = candidates.iter().map(series_key).collect::<Vec<_>>();
    keys.sort_by(|left, right| {
        left.0.cmp(&right.0).then_with(|| {
            let left_rank = left
                .1
                .as_ref()
                .and_then(|key| split_first.get(key))
                .copied()
                .unwrap_or(0);
            let right_rank = right
                .1
                .as_ref()
                .and_then(|key| split_first.get(key))
                .copied()
                .unwrap_or(0);
            left_rank.cmp(&right_rank)
        })
    });
    keys.dedup();
    keys
}

fn series_key(candidate: &Candidate) -> (usize, Option<String>) {
    (
        candidate.measure_index,
        candidate.split.as_ref().map(category_key),
    )
}

fn series_names(
    keys: &[(usize, Option<String>)],
    measures: &[ChartFieldRef],
) -> Result<Vec<String>, ChartError> {
    let split_values = keys
        .iter()
        .filter_map(|(_, key)| key.as_ref())
        .map(|key| serde_json::from_str::<CategoryValue>(key).expect("canonical category key"))
        .collect::<Vec<_>>();
    let mut split_index = 0;
    let mut names = Vec::with_capacity(keys.len());
    for (measure_index, split) in keys {
        let mut name = measures[*measure_index].display_name();
        if split.is_some() {
            name.push_str(" · ");
            name.push_str(&category_display(&split_values[split_index]));
            split_index += 1;
        }
        names.push(name);
    }
    let mut unique = HashSet::new();
    if names.iter().any(|name| !unique.insert(name.clone())) {
        return Err(ChartError::new(
            "chart_series_name_conflict",
            "图表系列名称冲突，请调整 label",
        ));
    }
    Ok(names)
}

fn common_unit(fields: &[ResolvedField]) -> Result<Option<String>, ChartError> {
    let first = fields.first().and_then(|field| field.unit.clone());
    if fields.iter().any(|field| field.unit != first) {
        return Err(ChartError::new(
            "chart_unit_conflict",
            "多个 measure 的单位不一致",
        ));
    }
    Ok(first)
}

fn category_key(value: &CategoryValue) -> String {
    serde_json::to_string(value).expect("CategoryValue serialization is infallible")
}

fn x_key(value: &LineXValue) -> String {
    match value {
        LineXValue::Category(value) => category_key(value),
        LineXValue::Number(value) => {
            let normalized = if *value == 0.0 { 0.0 } else { *value };
            format!("n:{:016x}", normalized.to_bits())
        }
    }
}

fn category_display(value: &CategoryValue) -> String {
    match value {
        CategoryValue::String { value } | CategoryValue::Integer { value } => value.clone(),
        CategoryValue::Boolean { value } => value.to_string(),
        CategoryValue::Number { value } => value.to_string(),
    }
}

fn model_base(
    source: &ChartSource,
    title: Option<String>,
    config_hash: String,
    diagnostics: ChartDiagnostics,
) -> ChartModelBase {
    ChartModelBase {
        contract_version: CHART_CONTRACT_VERSION.into(),
        view_id: None,
        source_dataset_id: source.dataset_id.clone(),
        source_deleted: false,
        name: None,
        title,
        config_hash,
        diagnostics,
    }
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
        _ => unreachable!("scalar compiler only returns line/bar/scatter"),
    }
}

fn line_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("ordinal", "UInt64", false),
        plot_column("x_kind", "Utf8", false),
        plot_column("x_category_json", "Utf8", true),
        plot_column("x_value", "Float64", true),
        plot_column("series", "Utf8", false),
        plot_column("value", "Float64", false),
    ]
}

fn bar_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("category_index", "UInt64", false),
        plot_column("category_json", "Utf8", false),
        plot_column("series", "Utf8", false),
        plot_column("value", "Float64", true),
    ]
}

fn scatter_plot_schema() -> Vec<DataFrameColumn> {
    vec![
        plot_column("ordinal", "UInt64", false),
        plot_column("series", "Utf8", false),
        plot_column("x", "Float64", false),
        plot_column("y", "Float64", false),
    ]
}

fn plot_column(name: &str, dtype: &str, nullable: bool) -> DataFrameColumn {
    DataFrameColumn {
        name: name.into(),
        dtype: dtype.into(),
        nullable,
        unit: None,
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
        let ChartModel::Line { series, y_axis, .. } = &compiled.model else {
            panic!("expected line model")
        };
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
}
