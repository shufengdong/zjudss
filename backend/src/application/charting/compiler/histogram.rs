use super::common::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    source: &ChartSource,
    title: Option<String>,
    presentation: ChartPresentation,
    value: ChartFieldRef,
    bin_count: u16,
    field: &ResolvedField,
    config_hash: String,
    mut diagnostics: ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<CompiledChart, ChartError> {
    let raw_values = source.batches.iter().map(RecordBatch::num_rows).sum();
    enforce_budget("rawValues", raw_values, budgets.max_raw_values)?;
    diagnostics.candidate_points = raw_values as u64;
    let mut values = Vec::with_capacity(raw_values);
    for batch in &source.batches {
        for row in 0..batch.num_rows() {
            let observed = field.read(batch, row)?;
            if !candidate_values_are_valid(std::slice::from_ref(&observed), &mut diagnostics) {
                continue;
            }
            values.push(observed.as_number()?);
        }
    }
    if values.is_empty() {
        return Err(no_data());
    }
    let minimum = values
        .iter()
        .copied()
        .min_by(f64::total_cmp)
        .expect("non-empty histogram");
    let maximum = values
        .iter()
        .copied()
        .max_by(f64::total_cmp)
        .expect("non-empty histogram");
    let bins = if minimum == maximum {
        vec![HistogramBin {
            start: minimum,
            end: maximum,
            count: values.len() as u64,
            includes_end: true,
        }]
    } else {
        let count = usize::from(bin_count);
        let width = (maximum - minimum) / count as f64;
        let mut counts = vec![0u64; count];
        for observed in values {
            let index = (((observed - minimum) / width).floor() as usize).min(count - 1);
            counts[index] += 1;
        }
        counts
            .into_iter()
            .enumerate()
            .map(|(index, count)| HistogramBin {
                start: minimum + index as f64 * width,
                end: if index + 1 == usize::from(bin_count) {
                    maximum
                } else {
                    minimum + (index + 1) as f64 * width
                },
                count,
                includes_end: index + 1 == usize::from(bin_count),
            })
            .collect()
    };
    diagnostics.aggregated_groups = bins.iter().filter(|bin| bin.count > 1).count() as u64;
    diagnostics.output_points = bins.len() as u64;
    enforce_budget("points", bins.len(), budgets.max_points)?;
    let base = model_base(source, title, config_hash, diagnostics.clone());
    let model = ChartModel::Histogram {
        base,
        presentation,
        value: HistogramValue {
            name: value.display_name(),
            unit: field.unit.clone(),
        },
        bins,
    };
    Ok(compiled_from_model(model, diagnostics))
}
