use super::common::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    source: &ChartSource,
    title: Option<String>,
    presentation: ChartPresentation,
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
