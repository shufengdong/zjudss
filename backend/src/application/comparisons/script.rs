use super::compatibility::CompatibilityFacts;

pub(super) fn render_script(compatibility: &CompatibilityFacts) -> String {
    let keys = &compatibility.stable_keys;
    let values = compatibility
        .values
        .iter()
        .map(|value| value.column.as_str())
        .collect::<Vec<_>>();
    let baseline_projection = keys
        .iter()
        .map(|column| format!("    col({column})"))
        .chain(
            values
                .iter()
                .map(|column| format!("    alias(col({column}), baseline_{column})")),
        )
        .collect::<Vec<_>>()
        .join(",\n");
    let candidate_projection = keys
        .iter()
        .map(|column| format!("    col({column})"))
        .chain(
            values
                .iter()
                .map(|column| format!("    alias(col({column}), candidate_{column})")),
        )
        .collect::<Vec<_>>()
        .join(",\n");
    let key_list = format!("[{}]", keys.join(", "));
    let deltas = values
        .iter()
        .map(|column| {
            format!("    alias(col(candidate_{column}) - col(baseline_{column}), delta_{column})")
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "baseline_selected = select(\n{baseline_projection},\n    baseline\n);\n\
candidate_selected = select(\n{candidate_projection},\n    candidate\n);\n\
aligned = join(baseline_selected, candidate_selected, {key_list}, {key_list}, \"full\");\n\
result = with_columns(\n{deltas},\n    aligned\n);\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::comparisons::compatibility::{CompatibilityFacts, CompatibleValue};

    #[test]
    fn uses_only_supported_frame_functions() {
        let facts = CompatibilityFacts {
            stable_keys: vec!["bus".into(), "phase".into()],
            values: vec![
                CompatibleValue::new("v_re", "Float64", Some("V")),
                CompatibleValue::new("v_im", "Float64", Some("V")),
            ],
            ..CompatibilityFacts::default()
        };
        let script = render_script(&facts);

        assert!(script.contains("baseline_selected = select("));
        assert!(script.contains("candidate_selected = select("));
        assert!(script.contains("aligned = join("));
        assert!(script.contains("result = with_columns("));
        assert!(script.contains("delta_v_re"));
        assert!(script.contains("delta_v_im"));
        assert!(!script.contains("compare_frames"));
        assert!(script.trim_end().ends_with(';'));
    }
}
