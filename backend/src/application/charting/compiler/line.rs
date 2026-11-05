use super::common::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    source: &ChartSource,
    title: Option<String>,
    presentation: LineChartPresentation,
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
        order,
        aggregation,
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
