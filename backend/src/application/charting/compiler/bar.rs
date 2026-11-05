use super::common::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    source: &ChartSource,
    title: Option<String>,
    presentation: ChartPresentation,
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
