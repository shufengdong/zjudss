use crate::application::datasets::DatasetColumn;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const VISUALIZATION_KINDS: &[&str] =
    &["line", "bar", "scatter", "heatmap", "boxplot", "histogram"];

#[derive(Debug, Clone, Copy)]
pub struct VisualizationBudgets {
    pub max_points: u64,
    pub max_series: u64,
    pub max_categories: u64,
    pub max_heatmap_cells: u64,
}
impl Default for VisualizationBudgets {
    fn default() -> Self {
        Self {
            max_points: 50_000,
            max_series: 100,
            max_categories: 5_000,
            max_heatmap_cells: 100_000,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisualizationSpec {
    pub kind: String,
    pub x: Option<String>,
    #[serde(default)]
    pub y: Vec<String>,
    pub series: Option<String>,
    pub value: Option<String>,
    pub category: Option<String>,
    pub bins: Option<u32>,
    pub title: Option<String>,
}
#[derive(Debug, Clone)]
pub struct ProjectionPlan {
    pub columns: Vec<String>,
    pub estimated_points: u64,
}
#[derive(Debug)]
pub struct VisualizationError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
}
impl VisualizationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }
    fn size(kind: &str, estimated: u64, limit: u64) -> Self {
        Self {
            code: "visualization_too_large",
            message: "可视化规模超过预算，请在对话中要求筛选或聚合".into(),
            details: Some(serde_json::json!({"kind":kind,"estimated":estimated,"limit":limit})),
        }
    }
}
impl std::fmt::Display for VisualizationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for VisualizationError {}
pub struct VisualizationValidator;
impl VisualizationValidator {
    pub fn parse_and_validate(
        schema: &[DatasetColumn],
        row_count: u64,
        value: &Value,
        budgets: VisualizationBudgets,
    ) -> Result<(VisualizationSpec, ProjectionPlan), VisualizationError> {
        if unsafe_value(value) {
            return Err(VisualizationError::new(
                "visualization_unsafe",
                "spec 禁止 formatter、函数或远程资源",
            ));
        }
        let spec: VisualizationSpec = serde_json::from_value(value.clone())
            .map_err(|e| VisualizationError::new("visualization_spec_invalid", e.to_string()))?;
        Self::validate(schema, row_count, &spec, budgets).map(|p| (spec, p))
    }
    pub fn validate(
        schema: &[DatasetColumn],
        rows: u64,
        s: &VisualizationSpec,
        b: VisualizationBudgets,
    ) -> Result<ProjectionPlan, VisualizationError> {
        if !VISUALIZATION_KINDS.contains(&s.kind.as_str()) {
            return Err(VisualizationError::new(
                "visualization_kind_invalid",
                &s.kind,
            ));
        }
        let map = schema
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect::<std::collections::HashMap<_, _>>();
        let mut cols = BTreeSet::new();
        let mut need = |v: &Option<String>, numeric: bool| -> Result<(), VisualizationError> {
            if let Some(name) = v {
                let c = map
                    .get(name.as_str())
                    .ok_or_else(|| VisualizationError::new("visualization_column_missing", name))?;
                if numeric && !is_numeric(&c.dtype) {
                    return Err(VisualizationError::new(
                        "visualization_dtype_invalid",
                        format!("{name} 不是数值列"),
                    ));
                }
                cols.insert(name.clone());
            }
            Ok(())
        };
        match s.kind.as_str() {
            "line" | "bar" | "scatter" => {
                need(&s.x, false)?;
                if s.x.is_none() || s.y.is_empty() {
                    return Err(VisualizationError::new(
                        "visualization_spec_invalid",
                        "需要 x 和至少一个 y",
                    ));
                }
                for y in &s.y {
                    need(&Some(y.clone()), true)?;
                }
                need(&s.series, false)?;
                let series = s.y.len() as u64;
                if series > b.max_series {
                    return Err(VisualizationError::size(&s.kind, series, b.max_series));
                }
                let points = rows.saturating_mul(series);
                if points > b.max_points {
                    return Err(VisualizationError::size(&s.kind, points, b.max_points));
                }
            }
            "heatmap" => {
                need(&s.x, false)?;
                need(&s.category, false)?;
                need(&s.value, true)?;
                if s.x.is_none() || s.category.is_none() || s.value.is_none() {
                    return Err(VisualizationError::new(
                        "visualization_spec_invalid",
                        "heatmap 需要 x/category/value",
                    ));
                }
                if rows > b.max_heatmap_cells {
                    return Err(VisualizationError::size(&s.kind, rows, b.max_heatmap_cells));
                }
            }
            "boxplot" => {
                need(&s.category, false)?;
                need(&s.value, true)?;
                if s.category.is_none() || s.value.is_none() {
                    return Err(VisualizationError::new(
                        "visualization_spec_invalid",
                        "boxplot 需要 category/value",
                    ));
                }
                if rows > b.max_points {
                    return Err(VisualizationError::size(&s.kind, rows, b.max_points));
                }
            }
            "histogram" => {
                need(&s.value, true)?;
                if s.value.is_none() || !(1..=200).contains(&s.bins.unwrap_or(20)) {
                    return Err(VisualizationError::new(
                        "visualization_spec_invalid",
                        "histogram 需要 value，bins 必须为 1..=200",
                    ));
                }
                if rows > b.max_points {
                    return Err(VisualizationError::size(&s.kind, rows, b.max_points));
                }
            }
            _ => unreachable!(),
        };
        if matches!(s.kind.as_str(), "bar" | "boxplot") && rows > b.max_categories {
            return Err(VisualizationError::size(&s.kind, rows, b.max_categories));
        }
        Ok(ProjectionPlan {
            columns: cols.into_iter().collect(),
            estimated_points: rows,
        })
    }
}
fn is_numeric(v: &str) -> bool {
    ["Int", "UInt", "Float", "Decimal"]
        .iter()
        .any(|p| v.starts_with(p))
        || [
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "int64", "float64",
        ]
        .contains(&v)
}
fn unsafe_value(v: &Value) -> bool {
    match v {
        Value::Object(m) => m
            .iter()
            .any(|(k, v)| k.to_ascii_lowercase().contains("formatter") || unsafe_value(v)),
        Value::Array(a) => a.iter().any(unsafe_value),
        Value::String(s) => {
            s.contains("http://")
                || s.contains("https://")
                || s.contains("javascript:")
                || s.contains("function(")
        }
        _ => false,
    }
}
