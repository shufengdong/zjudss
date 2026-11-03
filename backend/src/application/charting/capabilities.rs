use std::collections::BTreeSet;

use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::DataType;

use super::fields::list_values;
use super::{
    CapabilityReason, CapabilityRole, CapabilityScalarType, CapabilityShapeReason, ChartBudgets,
    ChartCapabilities, ChartCapabilityField, ChartCapabilityLimits, ChartError, ChartFieldShape,
    ChartKind, ChartTemplateCapability,
};
use crate::application::dataframe_reader::DataFrameColumn;

pub const CHART_CAPABILITY_SCAN_ROWS: usize = 10_000;

pub struct CapabilityInspector;

impl CapabilityInspector {
    pub fn inspect(
        dataset_id: impl Into<String>,
        schema_hash: impl Into<String>,
        columns: &[DataFrameColumn],
        batches: &[RecordBatch],
        total_rows: u64,
        budgets: ChartBudgets,
    ) -> Result<ChartCapabilities, ChartError> {
        if columns.len() > budgets.max_fields {
            return Err(capabilities_too_large(
                "fields",
                columns.len(),
                budgets.max_fields,
            ));
        }
        let fields = columns
            .iter()
            .map(|column| inspect_field(column, batches, total_rows, budgets))
            .collect::<Result<Vec<_>, _>>()?;
        let templates = template_capabilities(&fields);
        let result = ChartCapabilities {
            dataset_id: dataset_id.into(),
            schema_hash: schema_hash.into(),
            row_count: total_rows,
            fields,
            templates,
            limits: ChartCapabilityLimits {
                max_fields: budgets.max_fields,
                max_observed_paths_per_field: budgets.max_observed_paths_per_field,
                max_series: budgets.max_series,
                max_points: budgets.max_points,
                max_categories: budgets.max_categories,
                max_heatmap_cells: budgets.max_heatmap_cells,
            },
        };
        let bytes = serde_json::to_vec(&result).map_err(|error| {
            ChartError::new(
                "chart_materialization_failed",
                format!("图表 capability 无法序列化: {error}"),
            )
        })?;
        if bytes.len() > budgets.max_capability_response_bytes {
            return Err(capabilities_too_large(
                "responseBytes",
                bytes.len(),
                budgets.max_capability_response_bytes,
            ));
        }
        Ok(result)
    }
}

fn inspect_field(
    column: &DataFrameColumn,
    batches: &[RecordBatch],
    total_rows: u64,
    budgets: ChartBudgets,
) -> Result<ChartCapabilityField, ChartError> {
    let array = first_array(batches, &column.name);
    let data_type = array.map(|array| array.data_type());
    let (rank, leaf_type, variable) = match data_type {
        Some(data_type) => list_leaf(data_type),
        None => list_leaf_from_dtype(&column.dtype),
    };
    let scalar_type = scalar_type(leaf_type);
    let (shape, roles) = if rank == 0 {
        (ChartFieldShape::Scalar, scalar_roles(&scalar_type).to_vec())
    } else if rank > 4 || !is_numeric_scalar(leaf_type) {
        (
            ChartFieldShape::List {
                rank,
                observed_paths: Vec::new(),
                complete: false,
                truncated: false,
                reason: None,
            },
            Vec::new(),
        )
    } else {
        let discovery = if !variable {
            data_type
                .map(|data_type| fixed_paths(data_type, budgets.max_observed_paths_per_field))
                .unwrap_or_else(|| PathDiscovery::empty(false))
        } else {
            discover_paths(
                batches,
                &column.name,
                total_rows,
                budgets.max_observed_paths_per_field,
            )?
        };
        let roles = if discovery.truncated {
            Vec::new()
        } else {
            vec![CapabilityRole::Measure]
        };
        (
            ChartFieldShape::List {
                rank,
                observed_paths: if discovery.truncated {
                    Vec::new()
                } else {
                    discovery.paths
                },
                complete: discovery.complete,
                truncated: discovery.truncated,
                reason: discovery
                    .truncated
                    .then_some(CapabilityShapeReason::TooManyComponents),
            },
            roles,
        )
    };
    Ok(ChartCapabilityField {
        column: column.name.clone(),
        dtype: column.dtype.clone(),
        nullable: column.nullable,
        unit: column.unit.clone(),
        scalar_type,
        roles,
        shape,
    })
}

fn first_array<'a>(batches: &'a [RecordBatch], name: &str) -> Option<&'a ArrayRef> {
    batches.iter().find_map(|batch| {
        batch
            .schema()
            .index_of(name)
            .ok()
            .map(|index| batch.column(index))
    })
}

fn list_leaf(data_type: &DataType) -> (usize, &DataType, bool) {
    let mut rank = 0;
    let mut current = data_type;
    let mut variable = false;
    loop {
        match current {
            DataType::FixedSizeList(field, _) => {
                rank += 1;
                current = field.data_type();
            }
            DataType::List(field)
            | DataType::LargeList(field)
            | DataType::ListView(field)
            | DataType::LargeListView(field) => {
                rank += 1;
                variable = true;
                current = field.data_type();
            }
            _ => return (rank, current, variable),
        }
    }
}

fn list_leaf_from_dtype(dtype: &str) -> (usize, &DataType, bool) {
    if dtype.contains("List") {
        (1, &DataType::Null, true)
    } else {
        (0, scalar_data_type_from_name(dtype), false)
    }
}

fn scalar_data_type_from_name(dtype: &str) -> &'static DataType {
    match dtype {
        "Utf8" => &DataType::Utf8,
        "LargeUtf8" => &DataType::LargeUtf8,
        "Utf8View" => &DataType::Utf8View,
        "Boolean" => &DataType::Boolean,
        "Int8" => &DataType::Int8,
        "Int16" => &DataType::Int16,
        "Int32" => &DataType::Int32,
        "Int64" => &DataType::Int64,
        "UInt8" => &DataType::UInt8,
        "UInt16" => &DataType::UInt16,
        "UInt32" => &DataType::UInt32,
        "UInt64" => &DataType::UInt64,
        "Float32" => &DataType::Float32,
        "Float64" => &DataType::Float64,
        _ => &DataType::Null,
    }
}

fn scalar_type(data_type: &DataType) -> CapabilityScalarType {
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => CapabilityScalarType::String,
        DataType::Boolean => CapabilityScalarType::Boolean,
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => CapabilityScalarType::Integer,
        DataType::Float32 | DataType::Float64 => CapabilityScalarType::Number,
        _ => CapabilityScalarType::Unsupported,
    }
}

fn scalar_roles(scalar_type: &CapabilityScalarType) -> &'static [CapabilityRole] {
    match scalar_type {
        CapabilityScalarType::String | CapabilityScalarType::Boolean => &[CapabilityRole::Category],
        CapabilityScalarType::Integer | CapabilityScalarType::Number => {
            &[CapabilityRole::Category, CapabilityRole::Measure]
        }
        CapabilityScalarType::Unsupported => &[],
    }
}

fn is_numeric_scalar(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    )
}

#[derive(Debug)]
struct PathDiscovery {
    paths: Vec<Vec<u16>>,
    complete: bool,
    truncated: bool,
}

impl PathDiscovery {
    fn empty(complete: bool) -> Self {
        Self {
            paths: Vec::new(),
            complete,
            truncated: false,
        }
    }
}

fn fixed_paths(data_type: &DataType, limit: usize) -> PathDiscovery {
    let mut paths = Vec::new();
    let mut path = Vec::new();
    let truncated = enumerate_fixed(data_type, &mut path, &mut paths, limit);
    PathDiscovery {
        paths,
        complete: true,
        truncated,
    }
}

fn enumerate_fixed(
    data_type: &DataType,
    path: &mut Vec<u16>,
    paths: &mut Vec<Vec<u16>>,
    limit: usize,
) -> bool {
    match data_type {
        DataType::FixedSizeList(field, length) => {
            let Ok(length) = u16::try_from(*length) else {
                return true;
            };
            if length > 1024 {
                return true;
            }
            for index in 0..length {
                path.push(index);
                if enumerate_fixed(field.data_type(), path, paths, limit) {
                    return true;
                }
                path.pop();
            }
            false
        }
        _ => {
            paths.push(path.clone());
            paths.len() > limit
        }
    }
}

fn discover_paths(
    batches: &[RecordBatch],
    column: &str,
    total_rows: u64,
    limit: usize,
) -> Result<PathDiscovery, ChartError> {
    let mut paths = BTreeSet::new();
    let mut scanned = 0_usize;
    let mut truncated = false;
    'batches: for batch in batches {
        let index = batch.schema().index_of(column).map_err(|_| {
            ChartError::new("chart_column_missing", "capability 扫描缺少字段")
                .with_details(serde_json::json!({"column": column}))
        })?;
        let array = batch.column(index);
        for row in 0..batch.num_rows() {
            if scanned == CHART_CAPABILITY_SCAN_ROWS {
                break 'batches;
            }
            scanned += 1;
            if array.is_null(row) {
                continue;
            }
            let mut path = Vec::new();
            if enumerate_row(array.as_ref(), row, &mut path, &mut paths, limit)? {
                truncated = true;
                break 'batches;
            }
        }
    }
    Ok(PathDiscovery {
        paths: paths.into_iter().collect(),
        complete: !truncated && total_rows <= scanned as u64,
        truncated,
    })
}

fn enumerate_row(
    array: &dyn Array,
    row: usize,
    path: &mut Vec<u16>,
    paths: &mut BTreeSet<Vec<u16>>,
    limit: usize,
) -> Result<bool, ChartError> {
    if array.is_null(row) {
        return Ok(false);
    }
    match array.data_type() {
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_)
        | DataType::FixedSizeList(_, _) => {
            if path.len() == 4 {
                return Ok(true);
            }
            let values = list_values(array, row)?;
            if values.len() > 1024 {
                return Ok(true);
            }
            for index in 0..values.len() {
                path.push(index as u16);
                if enumerate_row(values.as_ref(), index, path, paths, limit)? {
                    return Ok(true);
                }
                path.pop();
            }
            Ok(false)
        }
        data_type if is_numeric_scalar(data_type) => {
            paths.insert(path.clone());
            Ok(paths.len() > limit)
        }
        _ => Ok(false),
    }
}

fn template_capabilities(fields: &[ChartCapabilityField]) -> Vec<ChartTemplateCapability> {
    let has_category = fields
        .iter()
        .any(|field| field.roles.contains(&CapabilityRole::Category));
    let has_measure = fields
        .iter()
        .any(|field| field.roles.contains(&CapabilityRole::Measure));
    let has_truncated = fields.iter().any(|field| {
        matches!(
            field.shape,
            ChartFieldShape::List {
                truncated: true,
                ..
            }
        )
    });
    [
        ChartKind::Line,
        ChartKind::Bar,
        ChartKind::Scatter,
        ChartKind::Heatmap,
        ChartKind::Boxplot,
        ChartKind::Histogram,
    ]
    .into_iter()
    .map(|kind| {
        let needs_category = matches!(
            kind,
            ChartKind::Line | ChartKind::Bar | ChartKind::Heatmap | ChartKind::Boxplot
        );
        let enabled = has_measure && (!needs_category || has_category);
        let reason = if enabled {
            None
        } else if !has_measure && has_truncated {
            Some(CapabilityReason::CapabilityTruncated)
        } else if !has_measure {
            Some(CapabilityReason::NoNumericField)
        } else {
            Some(CapabilityReason::NoCategoryField)
        };
        ChartTemplateCapability {
            kind,
            enabled,
            reason,
        }
    })
    .collect()
}

fn capabilities_too_large(kind: &str, actual: usize, limit: usize) -> ChartError {
    ChartError::new(
        "chart_capabilities_too_large",
        "图表字段能力响应超过公共限制",
    )
    .with_details(serde_json::json!({"kind": kind, "actual": actual, "limit": limit}))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::builder::{Float64Builder, LargeListBuilder};
    use arrow_array::{
        ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringViewArray, UInt64Array,
    };

    use super::*;
    use crate::application::charting::{
        CapabilityReason, CapabilityRole, CapabilityScalarType, ChartBudgets, ChartFieldShape,
        ChartKind,
    };
    use crate::application::dataframe_reader::DataFrameColumn;

    fn column(name: &str, dtype: &str) -> DataFrameColumn {
        DataFrameColumn {
            name: name.into(),
            dtype: dtype.into(),
            nullable: true,
            unit: None,
        }
    }

    fn phases(rows: &[usize]) -> ArrayRef {
        let mut builder = LargeListBuilder::new(Float64Builder::new());
        let mut value = 0.0;
        for length in rows {
            for _ in 0..*length {
                builder.values().append_value(value);
                value += 1.0;
            }
            builder.append(true);
        }
        Arc::new(builder.finish())
    }

    #[test]
    fn scalar_roles_and_variable_list_paths_are_deterministic() {
        let batch = RecordBatch::try_from_iter(vec![
            (
                "bus",
                Arc::new(StringViewArray::from(vec!["b1", "b2"])) as ArrayRef,
            ),
            (
                "enabled",
                Arc::new(BooleanArray::from(vec![true, false])) as ArrayRef,
            ),
            ("signed", Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef),
            (
                "unsigned",
                Arc::new(UInt64Array::from(vec![3, 4])) as ArrayRef,
            ),
            (
                "number",
                Arc::new(Float64Array::from(vec![1.0, 2.0])) as ArrayRef,
            ),
            ("v_mag", phases(&[3, 2])),
        ])
        .unwrap();
        let columns = vec![
            column("bus", "Utf8View"),
            column("enabled", "Boolean"),
            column("signed", "Int64"),
            column("unsigned", "UInt64"),
            column("number", "Float64"),
            column("v_mag", "LargeList(Float64)"),
        ];

        let result = CapabilityInspector::inspect(
            "7",
            "schema-hash",
            &columns,
            &[batch],
            2,
            ChartBudgets::default(),
        )
        .unwrap();
        assert_eq!(result.dataset_id, "7");
        assert_eq!(result.schema_hash, "schema-hash");
        assert_eq!(result.fields[0].scalar_type, CapabilityScalarType::String);
        assert_eq!(result.fields[0].roles, vec![CapabilityRole::Category]);
        assert_eq!(result.fields[1].scalar_type, CapabilityScalarType::Boolean);
        assert_eq!(result.fields[2].scalar_type, CapabilityScalarType::Integer);
        assert_eq!(
            result.fields[2].roles,
            vec![CapabilityRole::Category, CapabilityRole::Measure]
        );
        assert_eq!(result.fields[4].scalar_type, CapabilityScalarType::Number);
        assert_eq!(
            result.fields[5].shape,
            ChartFieldShape::List {
                rank: 1,
                observed_paths: vec![vec![0], vec![1], vec![2]],
                complete: true,
                truncated: false,
                reason: None,
            }
        );
        assert!(result.templates.iter().all(|template| template.enabled));
    }

    #[test]
    fn discovery_stops_at_ten_thousand_rows_and_marks_incomplete() {
        let batch = RecordBatch::try_from_iter(vec![("v_mag", phases(&[3; 10_001]))]).unwrap();
        let result = CapabilityInspector::inspect(
            "7",
            "hash",
            &[column("v_mag", "LargeList(Float64)")],
            &[batch],
            10_001,
            ChartBudgets::default(),
        )
        .unwrap();
        let ChartFieldShape::List {
            observed_paths,
            complete,
            ..
        } = &result.fields[0].shape
        else {
            panic!("expected list capability")
        };
        assert_eq!(
            observed_paths.as_slice(),
            &[vec![0_u16], vec![1_u16], vec![2_u16]]
        );
        assert!(!complete);
    }

    #[test]
    fn oversized_component_fields_are_disabled_instead_of_exposing_a_prefix() {
        let batch = RecordBatch::try_from_iter(vec![("wide", phases(&[65]))]).unwrap();
        let columns = vec![column("wide", "LargeList(Float64)")];
        let result = CapabilityInspector::inspect(
            "7",
            "hash",
            &columns,
            &[batch],
            1,
            ChartBudgets::default(),
        )
        .unwrap();
        let ChartFieldShape::List {
            observed_paths,
            truncated,
            ..
        } = &result.fields[0].shape
        else {
            panic!("expected list capability")
        };
        assert!(observed_paths.is_empty());
        assert!(truncated);
        let histogram = result
            .templates
            .iter()
            .find(|template| template.kind == ChartKind::Histogram)
            .unwrap();
        assert!(!histogram.enabled);
        assert_eq!(
            histogram.reason,
            Some(CapabilityReason::CapabilityTruncated)
        );
    }

    #[test]
    fn field_count_and_serialized_response_budgets_fail_closed() {
        let batch = RecordBatch::try_from_iter(vec![
            ("a", Arc::new(Float64Array::from(vec![1.0])) as ArrayRef),
            ("b", Arc::new(Float64Array::from(vec![2.0])) as ArrayRef),
        ])
        .unwrap();
        let columns = vec![column("a", "Float64"), column("b", "Float64")];
        let mut budgets = ChartBudgets::default();
        budgets.max_fields = 1;
        assert_eq!(
            CapabilityInspector::inspect("7", "hash", &columns, &[batch.clone()], 1, budgets)
                .unwrap_err()
                .code,
            "chart_capabilities_too_large"
        );

        let mut budgets = ChartBudgets::default();
        budgets.max_capability_response_bytes = 1;
        assert_eq!(
            CapabilityInspector::inspect("7", "hash", &columns[..1], &[batch], 1, budgets)
                .unwrap_err()
                .code,
            "chart_capabilities_too_large"
        );
    }
}
