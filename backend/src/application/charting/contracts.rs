use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::ChartError;

pub const CHART_CONTRACT_VERSION: &str = "chart-model-v1";
pub const VISUALIZATION_KINDS: &[&str] =
    &["line", "bar", "scatter", "heatmap", "boxplot", "histogram"];

#[derive(Debug, Clone, Copy)]
pub struct ChartBudgets {
    pub max_scan_rows: u64,
    pub max_referenced_columns: usize,
    pub max_measures: usize,
    pub max_series: usize,
    pub max_raw_values: usize,
    pub max_points: usize,
    pub max_categories: usize,
    pub max_heatmap_cells: usize,
    pub max_response_bytes: usize,
    pub max_fields: usize,
    pub max_observed_paths_per_field: usize,
    pub max_capability_response_bytes: usize,
}

impl Default for ChartBudgets {
    fn default() -> Self {
        Self {
            max_scan_rows: 1_000_000,
            max_referenced_columns: 32,
            max_measures: 16,
            max_series: 100,
            max_raw_values: 500_000,
            max_points: 50_000,
            max_categories: 5_000,
            max_heatmap_cells: 100_000,
            max_response_bytes: 10 * 1024 * 1024,
            max_fields: 256,
            max_observed_paths_per_field: 64,
            max_capability_response_bytes: 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    Line,
    Bar,
    Scatter,
    Heatmap,
    Boxplot,
    Histogram,
}

impl ChartKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Bar => "bar",
            Self::Scatter => "scatter",
            Self::Heatmap => "heatmap",
            Self::Boxplot => "boxplot",
            Self::Histogram => "histogram",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartFieldRef {
    pub column: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl ChartFieldRef {
    pub fn display_name(&self) -> String {
        self.label.clone().unwrap_or_else(|| canonical_label(self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendPosition {
    Auto,
    Top,
    Bottom,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LegendOrientation {
    Auto,
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartPalette {
    Default,
    Colorblind,
    HighContrast,
    Monochrome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineStyle {
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegendPresentation {
    pub show: bool,
    pub position: LegendPosition,
    pub orientation: LegendOrientation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TitlePresentation {
    pub show: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AxisPresentation {
    pub show_grid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinePresentation {
    pub width: u8,
    pub style: LineStyle,
    pub show_symbol: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartPresentation {
    pub legend: LegendPresentation,
    pub palette: ChartPalette,
    pub title: TitlePresentation,
    pub axis: AxisPresentation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LineChartPresentation {
    pub legend: LegendPresentation,
    pub palette: ChartPalette,
    pub title: TitlePresentation,
    pub axis: AxisPresentation,
    pub line: LinePresentation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AxisMode {
    Category,
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartOrder {
    Source,
    XAsc,
    XDesc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Aggregation {
    None,
    Mean,
    Min,
    Max,
    Sum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HeatmapAggregation {
    Mean,
    Min,
    Max,
    Sum,
    Count,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChartSampling {
    None {},
    Lttb { max_points_per_series: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ChartConfig {
    Line {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        presentation: LineChartPresentation,
        x: ChartFieldRef,
        x_axis_mode: AxisMode,
        measures: Vec<ChartFieldRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        split_by: Option<ChartFieldRef>,
        order: ChartOrder,
        aggregation: Aggregation,
        sampling: ChartSampling,
    },
    Bar {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        presentation: ChartPresentation,
        x: ChartFieldRef,
        measures: Vec<ChartFieldRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        split_by: Option<ChartFieldRef>,
        order: ChartOrder,
        aggregation: Aggregation,
    },
    Scatter {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        presentation: ChartPresentation,
        x: ChartFieldRef,
        measures: Vec<ChartFieldRef>,
        #[serde(skip_serializing_if = "Option::is_none")]
        split_by: Option<ChartFieldRef>,
    },
    Heatmap {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        presentation: ChartPresentation,
        x: ChartFieldRef,
        y: ChartFieldRef,
        value: ChartFieldRef,
        aggregation: HeatmapAggregation,
    },
    Boxplot {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        presentation: ChartPresentation,
        category: ChartFieldRef,
        value: ChartFieldRef,
        #[serde(skip_serializing_if = "Option::is_none")]
        split_by: Option<ChartFieldRef>,
        show_outliers: bool,
    },
    Histogram {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        presentation: ChartPresentation,
        value: ChartFieldRef,
        bins: u16,
    },
}

impl ChartConfig {
    pub fn kind(&self) -> ChartKind {
        match self {
            Self::Line { .. } => ChartKind::Line,
            Self::Bar { .. } => ChartKind::Bar,
            Self::Scatter { .. } => ChartKind::Scatter,
            Self::Heatmap { .. } => ChartKind::Heatmap,
            Self::Boxplot { .. } => ChartKind::Boxplot,
            Self::Histogram { .. } => ChartKind::Histogram,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CategoryValue {
    String { value: String },
    Boolean { value: bool },
    Integer { value: String },
    Number { value: f64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartDiagnostics {
    pub input_rows: u64,
    pub candidate_points: u64,
    pub output_points: u64,
    pub dropped_missing: u64,
    pub dropped_non_finite: u64,
    pub missing_components: u64,
    pub aggregated_groups: u64,
    pub sampled_points: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityScalarType {
    String,
    Boolean,
    Integer,
    Number,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityRole {
    Category,
    Measure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityReason {
    NoCategoryField,
    NoNumericField,
    CapabilityTruncated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityShapeReason {
    TooManyComponents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ChartFieldShape {
    Scalar,
    List {
        rank: usize,
        observed_paths: Vec<Vec<u16>>,
        complete: bool,
        truncated: bool,
        reason: Option<CapabilityShapeReason>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartCapabilityField {
    pub column: String,
    pub dtype: String,
    pub nullable: bool,
    pub unit: Option<String>,
    pub scalar_type: CapabilityScalarType,
    pub roles: Vec<CapabilityRole>,
    pub shape: ChartFieldShape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartTemplateCapability {
    pub kind: ChartKind,
    pub enabled: bool,
    pub reason: Option<CapabilityReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartCapabilityLimits {
    pub max_fields: usize,
    pub max_observed_paths_per_field: usize,
    pub max_series: usize,
    pub max_points: usize,
    pub max_categories: usize,
    pub max_heatmap_cells: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartCapabilities {
    pub dataset_id: String,
    pub schema_hash: String,
    pub row_count: u64,
    pub fields: Vec<ChartCapabilityField>,
    pub templates: Vec<ChartTemplateCapability>,
    pub limits: ChartCapabilityLimits,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChartModelBase {
    pub contract_version: String,
    pub view_id: Option<String>,
    pub source_dataset_id: String,
    pub source_deleted: bool,
    pub name: Option<String>,
    pub title: Option<String>,
    pub config_hash: String,
    pub diagnostics: ChartDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CategoryAxis {
    pub r#type: CategoryAxisType,
    pub name: String,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryAxisType {
    Category,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MixedAxis {
    pub r#type: AxisMode,
    pub name: String,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValueAxis {
    pub name: Option<String>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LineXValue {
    Category(CategoryValue),
    Number(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinePoint {
    pub x: LineXValue,
    pub y: f64,
    pub ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NumericPoint {
    pub x: f64,
    pub y: f64,
    pub ordinal: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LineSeries {
    pub name: String,
    pub points: Vec<LinePoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScatterSeries {
    pub name: String,
    pub points: Vec<NumericPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BarSeries {
    pub name: String,
    pub values: Vec<Option<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeatmapValue {
    pub name: String,
    pub unit: Option<String>,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HeatmapCell {
    pub x_index: usize,
    pub y_index: usize,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedCategoryAxis {
    pub name: String,
    pub categories: Vec<CategoryValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedValueAxis {
    pub name: String,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoxplotOutlier {
    pub category_index: usize,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoxplotSeries {
    pub name: String,
    pub values: Vec<Option<[f64; 5]>>,
    pub outliers: Vec<BoxplotOutlier>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistogramValue {
    pub name: String,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistogramBin {
    pub start: f64,
    pub end: f64,
    pub count: u64,
    pub includes_end: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum ChartModel {
    Line {
        #[serde(flatten)]
        base: ChartModelBase,
        presentation: LineChartPresentation,
        x_axis: MixedAxis,
        y_axis: ValueAxis,
        series: Vec<LineSeries>,
    },
    Bar {
        #[serde(flatten)]
        base: ChartModelBase,
        presentation: ChartPresentation,
        x_axis: CategoryAxis,
        y_axis: ValueAxis,
        categories: Vec<CategoryValue>,
        series: Vec<BarSeries>,
    },
    Scatter {
        #[serde(flatten)]
        base: ChartModelBase,
        presentation: ChartPresentation,
        x_axis: MixedAxis,
        y_axis: ValueAxis,
        series: Vec<ScatterSeries>,
    },
    Heatmap {
        #[serde(flatten)]
        base: ChartModelBase,
        presentation: ChartPresentation,
        x_axis: NamedCategoryAxis,
        y_axis: NamedCategoryAxis,
        x_categories: Vec<CategoryValue>,
        y_categories: Vec<CategoryValue>,
        value: HeatmapValue,
        cells: Vec<HeatmapCell>,
    },
    Boxplot {
        #[serde(flatten)]
        base: ChartModelBase,
        presentation: ChartPresentation,
        category_axis: NamedCategoryAxis,
        value_axis: NamedValueAxis,
        series: Vec<BoxplotSeries>,
    },
    Histogram {
        #[serde(flatten)]
        base: ChartModelBase,
        presentation: ChartPresentation,
        value: HistogramValue,
        bins: Vec<HistogramBin>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedCreateChart {
    pub name: String,
    pub config: ChartConfig,
    pub config_json: String,
    pub config_hash: String,
    pub request_hash: String,
}

pub fn normalize_create_request(
    name: String,
    config: ChartConfig,
) -> Result<NormalizedCreateChart, ChartError> {
    let name = normalize_required_text(name, 100, "name")?;
    let (config, config_json, config_hash) = normalize_chart_config(config)?;
    let request_hash = hash_parts(&[
        CHART_CONTRACT_VERSION,
        "request",
        name.as_str(),
        config_json.as_str(),
    ]);
    Ok(NormalizedCreateChart {
        name,
        config,
        config_json,
        config_hash,
        request_hash,
    })
}

pub(crate) fn normalize_chart_config(
    mut config: ChartConfig,
) -> Result<(ChartConfig, String, String), ChartError> {
    normalize_config(&mut config)?;
    let config_json = serde_json::to_string(&config).map_err(|error| {
        ChartError::new(
            "chart_config_invalid",
            format!("图表配置无法序列化: {error}"),
        )
    })?;
    let config_hash = hash_parts(&[CHART_CONTRACT_VERSION, "config", &config_json]);
    Ok((config, config_json, config_hash))
}

fn normalize_config(config: &mut ChartConfig) -> Result<(), ChartError> {
    match config {
        ChartConfig::Line {
            version,
            title,
            presentation,
            x,
            x_axis_mode,
            measures,
            split_by,
            sampling,
            ..
        } => {
            validate_version(*version)?;
            normalize_title(title)?;
            validate_line_presentation(presentation)?;
            normalize_field(x)?;
            normalize_measures(measures)?;
            normalize_optional_field(split_by)?;
            match sampling {
                ChartSampling::None {} => {}
                ChartSampling::Lttb {
                    max_points_per_series,
                } => {
                    if *x_axis_mode != AxisMode::Value {
                        return Err(config_error("LTTB 只允许数值横轴折线图使用"));
                    }
                    if !(100..=10_000).contains(max_points_per_series) {
                        return Err(config_error("maxPointsPerSeries 必须为 100..=10000"));
                    }
                }
            }
        }
        ChartConfig::Bar {
            version,
            title,
            presentation,
            x,
            measures,
            split_by,
            ..
        }
        | ChartConfig::Scatter {
            version,
            title,
            presentation,
            x,
            measures,
            split_by,
        } => {
            validate_version(*version)?;
            normalize_title(title)?;
            validate_presentation(presentation)?;
            normalize_field(x)?;
            normalize_measures(measures)?;
            normalize_optional_field(split_by)?;
        }
        ChartConfig::Heatmap {
            version,
            title,
            presentation,
            x,
            y,
            value,
            ..
        } => {
            validate_version(*version)?;
            normalize_title(title)?;
            validate_presentation(presentation)?;
            normalize_field(x)?;
            normalize_field(y)?;
            normalize_field(value)?;
        }
        ChartConfig::Boxplot {
            version,
            title,
            presentation,
            category,
            value,
            split_by,
            ..
        } => {
            validate_version(*version)?;
            normalize_title(title)?;
            validate_presentation(presentation)?;
            normalize_field(category)?;
            normalize_field(value)?;
            normalize_optional_field(split_by)?;
        }
        ChartConfig::Histogram {
            version,
            title,
            presentation,
            value,
            bins,
        } => {
            validate_version(*version)?;
            normalize_title(title)?;
            validate_presentation(presentation)?;
            normalize_field(value)?;
            if !(1..=200).contains(bins) {
                return Err(config_error("bins 必须为 1..=200"));
            }
        }
    }
    Ok(())
}

fn validate_version(version: u8) -> Result<(), ChartError> {
    if version == 1 {
        Ok(())
    } else {
        Err(config_error("version 必须为 1"))
    }
}

fn normalize_title(title: &mut Option<String>) -> Result<(), ChartError> {
    let Some(value) = title.take() else {
        return Ok(());
    };
    let value = value.trim().to_owned();
    if value.chars().count() > 200 {
        return Err(config_error("title 最多 200 个字符"));
    }
    *title = if value.is_empty() { None } else { Some(value) };
    Ok(())
}

fn normalize_optional_field(field: &mut Option<ChartFieldRef>) -> Result<(), ChartError> {
    if let Some(field) = field {
        normalize_field(field)?;
    }
    Ok(())
}

fn normalize_measures(measures: &mut [ChartFieldRef]) -> Result<(), ChartError> {
    if measures.is_empty() || measures.len() > 16 {
        return Err(config_error("measures 数量必须为 1..=16"));
    }
    let mut labels = HashSet::new();
    for measure in measures {
        normalize_field(measure)?;
        let label = measure
            .label
            .as_deref()
            .expect("normalization always provides a label");
        if !labels.insert(label.to_owned()) {
            return Err(ChartError::new(
                "chart_series_name_conflict",
                "指标展示名重复，请为每个系列使用不同名称",
            )
            .with_details(serde_json::json!({"label": label})));
        }
    }
    Ok(())
}

fn normalize_field(field: &mut ChartFieldRef) -> Result<(), ChartError> {
    let column_length = field.column.chars().count();
    if column_length == 0 || column_length > 256 {
        return Err(config_error("column 长度必须为 1..=256"));
    }
    if let Some(path) = &field.path
        && (path.is_empty() || path.len() > 4 || path.iter().any(|index| *index > 1023))
    {
        return Err(config_error("path 必须包含 1..=4 个 0..=1023 索引"));
    }
    let label = match field.label.take() {
        Some(label) => normalize_required_text(label, 64, "label")?,
        None => canonical_label(field),
    };
    field.label = Some(label);
    Ok(())
}

fn canonical_label(field: &ChartFieldRef) -> String {
    let mut label = field.column.clone();
    if let Some(path) = &field.path {
        for index in path {
            label.push('[');
            label.push_str(&index.to_string());
            label.push(']');
        }
    }
    label
}

fn normalize_required_text(
    value: String,
    max_chars: usize,
    field: &'static str,
) -> Result<String, ChartError> {
    let value = value.trim().to_owned();
    let chars = value.chars().count();
    if chars == 0 || chars > max_chars {
        return Err(config_error(format!("{field} 长度必须为 1..={max_chars}")));
    }
    Ok(value)
}

fn validate_presentation(_presentation: &ChartPresentation) -> Result<(), ChartError> {
    Ok(())
}

fn validate_line_presentation(presentation: &LineChartPresentation) -> Result<(), ChartError> {
    if !(1..=4).contains(&presentation.line.width) {
        return Err(config_error("line.width 必须为 1..=4"));
    }
    Ok(())
}

fn config_error(message: impl Into<String>) -> ChartError {
    ChartError::new("chart_config_invalid", message)
}

fn hash_parts(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn presentation() -> Value {
        json!({
            "legend": {"show": true, "position": "auto", "orientation": "auto"},
            "palette": "colorblind",
            "title": {"show": true},
            "axis": {"showGrid": true}
        })
    }

    fn line_presentation() -> Value {
        let mut value = presentation();
        value["line"] = json!({"width": 2, "style": "solid", "showSymbol": false});
        value
    }

    #[test]
    fn six_chart_configs_reject_unknown_or_cross_kind_fields() {
        let configs = [
            json!({"version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},"xAxisMode":"category","measures":[{"column":"v_mag","path":[0]}],"order":"source","aggregation":"none","sampling":{"mode":"none"}}),
            json!({"version":1,"kind":"bar","presentation":presentation(),"x":{"column":"bus"},"measures":[{"column":"load"}],"order":"xAsc","aggregation":"mean"}),
            json!({"version":1,"kind":"scatter","presentation":presentation(),"x":{"column":"p"},"measures":[{"column":"q"}]}),
            json!({"version":1,"kind":"heatmap","presentation":presentation(),"x":{"column":"from"},"y":{"column":"to"},"value":{"column":"loading"},"aggregation":"max"}),
            json!({"version":1,"kind":"boxplot","presentation":presentation(),"category":{"column":"bus"},"value":{"column":"v"},"showOutliers":true}),
            json!({"version":1,"kind":"histogram","presentation":presentation(),"value":{"column":"v"},"bins":20}),
        ];
        for value in configs {
            serde_json::from_value::<ChartConfig>(value).expect("valid frozen chart config");
        }

        let unknown = json!({"version":1,"kind":"histogram","presentation":presentation(),"value":{"column":"v"},"bins":20,"x":{"column":"bus"}});
        assert!(serde_json::from_value::<ChartConfig>(unknown).is_err());
        let cross_kind = json!({"version":1,"kind":"bar","presentation":presentation(),"x":{"column":"bus"},"measures":[{"column":"load"}],"order":"source","aggregation":"none","sampling":{"mode":"none"}});
        assert!(serde_json::from_value::<ChartConfig>(cross_kind).is_err());
        let invalid_none = json!({"version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},"xAxisMode":"category","measures":[{"column":"load"}],"order":"source","aggregation":"none","sampling":{"mode":"none","maxPointsPerSeries":100}});
        assert!(serde_json::from_value::<ChartConfig>(invalid_none).is_err());
    }

    #[test]
    fn presentation_accepts_only_the_frozen_palette_and_line_style_enums() {
        let missing = json!({"version":1,"kind":"histogram","value":{"column":"v"},"bins":20});
        assert!(serde_json::from_value::<ChartConfig>(missing).is_err());

        let mut invalid_palette = json!({"version":1,"kind":"histogram","presentation":presentation(),"value":{"column":"v"},"bins":20});
        invalid_palette["presentation"]["palette"] = json!("rainbow");
        assert!(serde_json::from_value::<ChartConfig>(invalid_palette).is_err());

        let mut invalid_style = json!({"version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"x"},"xAxisMode":"value","measures":[{"column":"y"}],"order":"xAsc","aggregation":"none","sampling":{"mode":"lttb","maxPointsPerSeries":100}});
        invalid_style["presentation"]["line"]["style"] = json!("smooth");
        assert!(serde_json::from_value::<ChartConfig>(invalid_style).is_err());

        let mut bar_line_style = json!({"version":1,"kind":"bar","presentation":presentation(),"x":{"column":"bus"},"measures":[{"column":"load"}],"order":"source","aggregation":"none"});
        bar_line_style["presentation"]["line"] =
            json!({"width":2,"style":"solid","showSymbol":false});
        assert!(serde_json::from_value::<ChartConfig>(bar_line_style).is_err());
    }

    #[test]
    fn normalization_trims_text_expands_labels_and_produces_stable_hashes() {
        let left: ChartConfig = serde_json::from_value(json!({
            "version":1,"kind":"line","title":"  Voltage  ","presentation":line_presentation(),
            "x":{"column":"bus"},"xAxisMode":"category",
            "measures":[{"column":"v_mag","path":[0],"label":"  A phase  "},{"column":"v_mag","path":[1]}],
            "order":"source","aggregation":"none","sampling":{"mode":"none"}
        })).unwrap();
        let right: ChartConfig = serde_json::from_value(json!({
            "version":1,"kind":"line","title":"Voltage","presentation":line_presentation(),
            "x":{"column":"bus","label":"bus"},"xAxisMode":"category",
            "measures":[{"column":"v_mag","path":[0],"label":"A phase"},{"column":"v_mag","path":[1],"label":"v_mag[1]"}],
            "order":"source","aggregation":"none","sampling":{"mode":"none"}
        })).unwrap();

        let left = normalize_create_request("  Three phase  ".into(), left).unwrap();
        let right = normalize_create_request("Three phase".into(), right).unwrap();
        assert_eq!(left.name, "Three phase");
        assert_eq!(left.config, right.config);
        assert_eq!(left.config_hash, right.config_hash);
        assert_eq!(left.request_hash, right.request_hash);
        assert_eq!(left.config_json, right.config_json);
        assert!(left.config_json.contains("v_mag[1]"));
        assert!(!left.config_json.contains("  Voltage  "));

        let duplicate: ChartConfig = serde_json::from_value(json!({
            "version":1,"kind":"line","presentation":line_presentation(),"x":{"column":"bus"},"xAxisMode":"category",
            "measures":[{"column":"a","label":"same"},{"column":"b","label":" same "}],
            "order":"source","aggregation":"none","sampling":{"mode":"none"}
        })).unwrap();
        assert_eq!(
            normalize_create_request("duplicate".into(), duplicate)
                .unwrap_err()
                .code,
            "chart_series_name_conflict"
        );
    }

    #[test]
    fn category_values_preserve_large_integer_identity() {
        let value = CategoryValue::Integer {
            value: i64::MAX.to_string(),
        };
        assert_eq!(
            serde_json::to_value(value).unwrap(),
            json!({"kind":"integer","value":"9223372036854775807"})
        );
    }
}
