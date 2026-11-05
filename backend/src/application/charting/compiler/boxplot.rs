use super::common::*;
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    source: &ChartSource,
    title: Option<String>,
    presentation: ChartPresentation,
    category: ChartFieldRef,
    value: ChartFieldRef,
    split_by: Option<ChartFieldRef>,
    show_outliers: bool,
    fields: &[ResolvedField],
    config_hash: String,
    mut diagnostics: ChartDiagnostics,
    budgets: ChartBudgets,
) -> Result<CompiledChart, ChartError> {
    if split_by.as_ref().is_some_and(|field| field.path.is_some()) {
        return Err(incompatible("splitBy 只能引用标量类别字段"));
    }
    let raw_values = source.batches.iter().map(RecordBatch::num_rows).sum();
    enforce_budget("rawValues", raw_values, budgets.max_raw_values)?;
    diagnostics.candidate_points = raw_values as u64;
    let mut candidates = Vec::with_capacity(raw_values);
    let mut ordinal = 0u64;
    for batch in &source.batches {
        for row in 0..batch.num_rows() {
            let category_value = fields[0].read(batch, row)?;
            let value_value = fields[1].read(batch, row)?;
            let split_value = fields
                .get(2)
                .map(|field| field.read(batch, row))
                .transpose()?;
            let mut values = vec![category_value, value_value];
            if let Some(split_value) = split_value {
                values.push(split_value);
            }
            if !candidate_values_are_valid(&values, &mut diagnostics) {
                ordinal += 1;
                continue;
            }
            let split = if values.len() == 3 {
                let split = values[2].as_category()?;
                if matches!(split, CategoryValue::Number { .. }) {
                    return Err(incompatible("splitBy 只允许字符串、布尔或整数类别字段"));
                }
                Some(split)
            } else {
                None
            };
            candidates.push(BoxCandidate {
                category: values[0].as_category()?,
                split,
                value: values[1].as_number()?,
                ordinal,
            });
            ordinal += 1;
        }
    }
    if candidates.is_empty() {
        return Err(no_data());
    }
    let categories = distinct_categories(candidates.iter().map(|candidate| &candidate.category));
    enforce_budget("categories", categories.len(), budgets.max_categories)?;
    let category_indices = category_indices(&categories);
    let series_keys = box_series_keys(&candidates);
    enforce_budget("series", series_keys.len(), budgets.max_series)?;
    let names = box_series_names(&series_keys, &value)?;
    let mut grouped = HashMap::<(Option<String>, String), Vec<f64>>::new();
    for candidate in &candidates {
        grouped
            .entry((
                candidate.split.as_ref().map(category_key),
                category_key(&candidate.category),
            ))
            .or_default()
            .push(candidate.value);
    }
    diagnostics.aggregated_groups =
        grouped.values().filter(|values| values.len() > 1).count() as u64;
    let mut series = Vec::with_capacity(series_keys.len());
    for (series_index, key) in series_keys.iter().enumerate() {
        let mut boxes = vec![None; categories.len()];
        let mut outliers = Vec::new();
        for category_value in &categories {
            let Some(values) = grouped.get(&(key.clone(), category_key(category_value))) else {
                continue;
            };
            let (summary, observed_outliers) = box_summary(values);
            let category_index = category_indices[&category_key(category_value)];
            boxes[category_index] = Some(summary);
            if show_outliers {
                outliers.extend(observed_outliers.into_iter().map(|value| BoxplotOutlier {
                    category_index,
                    value,
                }));
            }
        }
        series.push(BoxplotSeries {
            name: names[series_index].clone(),
            values: boxes,
            outliers,
        });
    }
    diagnostics.output_points = series
        .iter()
        .flat_map(|series| series.values.iter())
        .filter(|value| value.is_some())
        .count() as u64;
    ensure_output(&diagnostics)?;
    enforce_budget(
        "points",
        diagnostics.output_points as usize,
        budgets.max_points,
    )?;
    let base = model_base(source, title, config_hash, diagnostics.clone());
    let model = ChartModel::Boxplot {
        base,
        presentation,
        category_axis: NamedCategoryAxis {
            name: category.display_name(),
            categories: categories.clone(),
        },
        value_axis: NamedValueAxis {
            name: value.display_name(),
            unit: fields[1].unit.clone(),
        },
        series,
    };
    Ok(compiled_from_model(model, diagnostics))
}
