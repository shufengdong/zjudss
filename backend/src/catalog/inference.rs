use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::ingestion::{ColumnSchema, TableSchema};

use super::schema::{FrameSchema, SchemaCatalog, normalize_name};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateScore {
    pub table_kind: String,
    pub confidence: f64,
    pub matched_required_columns: Vec<String>,
    pub matched_optional_columns: Vec<String>,
    pub missing_required_columns: Vec<String>,
    pub type_conflicts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceEvidence {
    pub table_kind: Option<String>,
    pub confidence: f64,
    pub resolution: String,
    pub candidates: Vec<CandidateScore>,
    pub matched_required_columns: Vec<String>,
    pub matched_optional_columns: Vec<String>,
    pub missing_required_columns: Vec<String>,
    pub type_conflicts: Vec<String>,
}

pub fn infer_table(table: &TableSchema, catalog: &SchemaCatalog) -> InferenceEvidence {
    let columns: BTreeMap<String, &ColumnSchema> = table
        .columns
        .iter()
        .map(|column| (normalize_name(&column.name), column))
        .collect();
    let names: BTreeSet<String> = columns.keys().cloned().collect();
    let mut candidates: Vec<_> = catalog
        .frames
        .iter()
        .map(|frame| score(frame, &names, &columns))
        .collect();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| left.table_kind.cmp(&right.table_kind))
    });
    let best = candidates.first().cloned();
    let margin = best
        .as_ref()
        .map(|first| {
            first.confidence
                - candidates
                    .get(1)
                    .map(|second| second.confidence)
                    .unwrap_or(0.0)
        })
        .unwrap_or(0.0);
    let inferred = best.as_ref().is_some_and(|candidate| {
        candidate.confidence >= 0.90
            && candidate.missing_required_columns.is_empty()
            && candidate.type_conflicts.is_empty()
            && margin >= 0.10
    });
    match best {
        Some(best) => InferenceEvidence {
            table_kind: inferred.then(|| best.table_kind.clone()),
            confidence: best.confidence,
            resolution: if inferred { "inferred" } else { "pending" }.to_string(),
            candidates,
            matched_required_columns: best.matched_required_columns,
            matched_optional_columns: best.matched_optional_columns,
            missing_required_columns: best.missing_required_columns,
            type_conflicts: best.type_conflicts,
        },
        None => InferenceEvidence {
            table_kind: None,
            confidence: 0.0,
            resolution: "pending".to_string(),
            candidates,
            matched_required_columns: vec![],
            matched_optional_columns: vec![],
            missing_required_columns: vec![],
            type_conflicts: vec![],
        },
    }
}

pub fn infer_tables(tables: &[TableSchema], catalog: &SchemaCatalog) -> Vec<InferenceEvidence> {
    let mut evidence: Vec<_> = tables
        .iter()
        .map(|table| infer_table(table, catalog))
        .collect();
    let mut counts = HashMap::new();
    for item in &evidence {
        if let Some(kind) = &item.table_kind {
            *counts.entry(kind.clone()).or_insert(0_usize) += 1;
        }
    }
    for item in &mut evidence {
        if item
            .table_kind
            .as_ref()
            .is_some_and(|kind| counts.get(kind).copied().unwrap_or(0) > 1)
        {
            item.resolution = "pending".to_string();
        }
    }
    evidence
}

fn score(
    frame: &FrameSchema,
    names: &BTreeSet<String>,
    columns: &BTreeMap<String, &ColumnSchema>,
) -> CandidateScore {
    let matched_required: Vec<_> = frame
        .required
        .keys()
        .filter(|name| names.contains(*name))
        .cloned()
        .collect();
    let missing_required: Vec<_> = frame
        .required
        .keys()
        .filter(|name| !names.contains(*name))
        .cloned()
        .collect();
    let matched_optional: Vec<_> = frame
        .optional
        .keys()
        .filter(|name| names.contains(*name))
        .cloned()
        .collect();
    let required_coverage = matched_required.len() as f64 / frame.required.len() as f64;
    let optional_coverage = if frame.optional.is_empty() {
        1.0
    } else {
        matched_optional.len() as f64 / frame.optional.len() as f64
    };
    let stable_coverage = if frame.stable_keys.is_empty() {
        1.0
    } else {
        frame
            .stable_keys
            .iter()
            .filter(|key| names.contains(*key))
            .count() as f64
            / frame.stable_keys.len() as f64
    };
    let confidence = required_coverage * 0.75 + optional_coverage * 0.15 + stable_coverage * 0.10;
    let mut type_conflicts = Vec::new();
    for (name, expected) in frame.required.iter().chain(frame.optional.iter()) {
        if let Some(column) = columns.get(name) {
            if !dtype_compatible(&column.dtype, expected) {
                type_conflicts.push(format!(
                    "{}: expected {}, got {}",
                    column.name, expected, column.dtype
                ));
            }
        }
    }
    CandidateScore {
        table_kind: frame.kind.clone(),
        confidence,
        matched_required_columns: matched_required,
        matched_optional_columns: matched_optional,
        missing_required_columns: missing_required,
        type_conflicts,
    }
}

fn dtype_compatible(actual: &str, expected: &str) -> bool {
    let actual = actual.to_ascii_lowercase();
    let expected = expected.to_ascii_lowercase();
    if actual == expected {
        return true;
    }
    if actual == "unknown" || expected == "unknown" {
        return true;
    }
    if expected.contains("str") {
        return actual.contains("str") || actual.contains("utf8");
    }
    if expected.contains("bool") {
        return actual.contains("bool");
    }
    if expected.contains("f64") {
        return actual.contains("float") || actual.contains("double") || actual.contains("int");
    }
    if expected.contains("u32") || expected.contains("u64") {
        return actual.contains("int") || actual.contains("uint");
    }
    true
}
