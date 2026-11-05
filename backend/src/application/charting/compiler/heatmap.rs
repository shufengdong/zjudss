use super::common::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    source: &ChartSource,
    title: Option<String>,
    presentation: ChartPresentation,
    x: ChartFieldRef,
    y: ChartFieldRef,
    value: ChartFieldRef,
    aggregation: HeatmapAggregation,
    fields: &[ResolvedField],
    config_hash: String,
    mut diagnostics: ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<CompiledChart, ChartError> {
    let raw_values = source.batches.iter().map(RecordBatch::num_rows).sum();
    enforce_budget("rawValues", raw_values, budgets.max_raw_values)?;
    diagnostics.candidate_points = raw_values as u64;
    let mut candidates = Vec::with_capacity(raw_values);
    for batch in &source.batches {
        for row in 0..batch.num_rows() {
            let values = [
                fields[0].read(batch, row)?,
                fields[1].read(batch, row)?,
                fields[2].read(batch, row)?,
            ];
            if !candidate_values_are_valid(&values, &mut diagnostics) {
                continue;
            }
            candidates.push(HeatCandidate {
                x: values[0].as_category()?,
                y: values[1].as_category()?,
                value: values[2].as_number()?,
            });
        }
    }
    if candidates.is_empty() {
        return Err(no_data());
    }
    let x_categories = distinct_categories(candidates.iter().map(|candidate| &candidate.x));
    let y_categories = distinct_categories(candidates.iter().map(|candidate| &candidate.y));
    enforce_budget("categories", x_categories.len(), budgets.max_categories)?;
    enforce_budget("categories", y_categories.len(), budgets.max_categories)?;
    let x_indices = category_indices(&x_categories);
    let y_indices = category_indices(&y_categories);
    let mut positions = HashMap::<(String, String), usize>::new();
    let mut groups = Vec::<(HeatCandidate, usize, f64, f64, f64)>::new();
    for candidate in candidates {
        let key = (category_key(&candidate.x), category_key(&candidate.y));
        if let Some(position) = positions.get(&key).copied() {
            let group = &mut groups[position];
            group.1 += 1;
            group.2 += candidate.value;
            group.3 = group.3.min(candidate.value);
            group.4 = group.4.max(candidate.value);
        } else {
            let position = groups.len();
            positions.insert(key, position);
            let observed = candidate.value;
            groups.push((candidate, 1, observed, observed, observed));
        }
    }
    diagnostics.aggregated_groups = groups.iter().filter(|group| group.1 > 1).count() as u64;
    enforce_budget("heatmapCells", groups.len(), budgets.max_heatmap_cells)?;
    let cells = groups
        .into_iter()
        .map(|(candidate, count, sum, min, max)| HeatmapCell {
            x_index: x_indices[&category_key(&candidate.x)],
            y_index: y_indices[&category_key(&candidate.y)],
            value: match aggregation {
                HeatmapAggregation::Mean => sum / count as f64,
                HeatmapAggregation::Min => min,
                HeatmapAggregation::Max => max,
                HeatmapAggregation::Sum => sum,
                HeatmapAggregation::Count => count as f64,
            },
        })
        .collect::<Vec<_>>();
    diagnostics.output_points = cells.len() as u64;
    let min = cells
        .iter()
        .map(|cell| cell.value)
        .min_by(f64::total_cmp)
        .expect("non-empty heatmap");
    let max = cells
        .iter()
        .map(|cell| cell.value)
        .max_by(f64::total_cmp)
        .expect("non-empty heatmap");
    let base = model_base(source, title, config_hash, diagnostics.clone());
    let model = ChartModel::Heatmap {
        base,
        presentation,
        x_axis: NamedCategoryAxis {
            name: x.display_name(),
            categories: x_categories.clone(),
        },
        y_axis: NamedCategoryAxis {
            name: y.display_name(),
            categories: y_categories.clone(),
        },
        x_categories,
        y_categories,
        value: HeatmapValue {
            name: value.display_name(),
            unit: if aggregation == HeatmapAggregation::Count {
                None
            } else {
                fields[2].unit.clone()
            },
            min,
            max,
        },
        cells,
    };
    Ok(compiled_from_model(model, diagnostics))
}
