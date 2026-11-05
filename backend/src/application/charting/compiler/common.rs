use super::*;

pub(super) fn aggregate_candidates(
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

pub(super) fn order_candidates(candidates: &mut [Candidate], order: ChartOrder) {
    match order {
        ChartOrder::Source => candidates.sort_by_key(|candidate| candidate.ordinal),
        ChartOrder::XAsc => candidates.sort_by(compare_candidate_x),
        ChartOrder::XDesc => candidates.sort_by(|left, right| {
            compare_x(&right.x, &left.x).then_with(|| left.ordinal.cmp(&right.ordinal))
        }),
    }
}

pub(super) fn compare_candidate_x(left: &Candidate, right: &Candidate) -> Ordering {
    compare_x(&left.x, &right.x).then_with(|| left.ordinal.cmp(&right.ordinal))
}

pub(super) fn compare_x(left: &LineXValue, right: &LineXValue) -> Ordering {
    match (left, right) {
        (LineXValue::Number(left), LineXValue::Number(right)) => left.total_cmp(right),
        (LineXValue::Category(left), LineXValue::Category(right)) => {
            compare_categories(left, right)
        }
        (LineXValue::Category(_), LineXValue::Number(_)) => Ordering::Less,
        (LineXValue::Number(_), LineXValue::Category(_)) => Ordering::Greater,
    }
}

pub(super) fn compare_categories(left: &CategoryValue, right: &CategoryValue) -> Ordering {
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

pub(super) fn category_rank(value: &CategoryValue) -> u8 {
    match value {
        CategoryValue::String { .. } => 0,
        CategoryValue::Boolean { .. } => 1,
        CategoryValue::Integer { .. } => 2,
        CategoryValue::Number { .. } => 3,
    }
}

pub(super) fn series_keys(candidates: &[Candidate]) -> Vec<(usize, Option<String>)> {
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

pub(super) fn series_key(candidate: &Candidate) -> (usize, Option<String>) {
    (
        candidate.measure_index,
        candidate.split.as_ref().map(category_key),
    )
}

pub(super) fn series_names(
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

pub(super) fn common_unit(fields: &[ResolvedField]) -> Result<Option<String>, ChartError> {
    let first = fields.first().and_then(|field| field.unit.clone());
    if fields.iter().any(|field| field.unit != first) {
        return Err(ChartError::new(
            "chart_unit_conflict",
            "多个 measure 的单位不一致",
        ));
    }
    Ok(first)
}

pub(super) fn category_key(value: &CategoryValue) -> String {
    serde_json::to_string(value).expect("CategoryValue serialization is infallible")
}

pub(super) fn x_key(value: &LineXValue) -> String {
    match value {
        LineXValue::Category(value) => category_key(value),
        LineXValue::Number(value) => {
            let normalized = if *value == 0.0 { 0.0 } else { *value };
            format!("n:{:016x}", normalized.to_bits())
        }
    }
}

pub(super) fn category_display(value: &CategoryValue) -> String {
    match value {
        CategoryValue::String { value } | CategoryValue::Integer { value } => value.clone(),
        CategoryValue::Boolean { value } => value.to_string(),
        CategoryValue::Number { value } => value.to_string(),
    }
}

pub(super) fn model_base(
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
