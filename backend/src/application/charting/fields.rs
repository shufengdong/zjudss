use arrow_array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, Float32Array, Float64Array, Int8Array,
    Int16Array, Int32Array, Int64Array, LargeListArray, LargeListViewArray, LargeStringArray,
    ListArray, ListViewArray, RecordBatch, StringArray, StringViewArray, UInt8Array, UInt16Array,
    UInt32Array, UInt64Array,
};
use arrow_schema::DataType;

use super::{CategoryValue, ChartError, ChartFieldRef};
use crate::application::dataframe_reader::DataFrameColumn;

const MAX_SAFE_INTEGER: i128 = 9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Category(CategoryValue),
    Number(f64),
    Missing,
    MissingComponent,
    NonFinite,
}

impl FieldValue {
    pub fn as_number(&self) -> Result<f64, ChartError> {
        match self {
            Self::Number(value) if value.is_finite() => Ok(*value),
            Self::Category(CategoryValue::Integer { value }) => {
                let integer = value
                    .parse::<i128>()
                    .map_err(|_| incompatible("整数值无法无损转换到数值轴", Some("integer")))?;
                if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&integer) {
                    return Err(incompatible(
                        "整数超过 JavaScript 安全整数范围，只能作为类别",
                        Some("integer"),
                    ));
                }
                Ok(integer as f64)
            }
            _ => Err(incompatible("字段值不适合数值槽位", None)),
        }
    }

    pub fn as_category(&self) -> Result<CategoryValue, ChartError> {
        match self {
            Self::Category(value) => Ok(value.clone()),
            Self::Number(value) if value.is_finite() => Ok(CategoryValue::Number { value: *value }),
            _ => Err(incompatible("字段值不适合类别槽位", None)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedField {
    pub reference: ChartFieldRef,
    pub column_index: usize,
    pub dtype: DataType,
    pub unit: Option<String>,
}

impl ResolvedField {
    pub fn bind(
        columns: &[DataFrameColumn],
        batch: &RecordBatch,
        reference: ChartFieldRef,
    ) -> Result<Self, ChartError> {
        let column = columns
            .iter()
            .find(|column| column.name == reference.column)
            .ok_or_else(|| {
                ChartError::new("chart_column_missing", "图表字段不存在")
                    .with_details(serde_json::json!({"column": reference.column}))
            })?;
        let column_index = batch.schema().index_of(&reference.column).map_err(|_| {
            ChartError::new("chart_column_missing", "Parquet 投影缺少图表字段")
                .with_details(serde_json::json!({"column": reference.column}))
        })?;
        Ok(Self {
            reference,
            column_index,
            dtype: batch.column(column_index).data_type().clone(),
            unit: column.unit.clone(),
        })
    }

    pub fn read(&self, batch: &RecordBatch, row: usize) -> Result<FieldValue, ChartError> {
        resolve_field(
            batch.column(self.column_index).as_ref(),
            row,
            &self.reference,
        )
    }
}

pub fn resolve_field(
    array: &dyn Array,
    row: usize,
    field: &ChartFieldRef,
) -> Result<FieldValue, ChartError> {
    if row >= array.len() {
        return Err(ChartError::new(
            "chart_materialization_failed",
            "图表读取行索引越界",
        ));
    }
    resolve_path(array, row, field.path.as_deref().unwrap_or_default())
}

fn resolve_path(array: &dyn Array, row: usize, path: &[u16]) -> Result<FieldValue, ChartError> {
    if array.is_null(row) {
        return Ok(FieldValue::Missing);
    }
    if let Some((&component, remaining)) = path.split_first() {
        let values = list_values(array, row)?;
        let component = usize::from(component);
        if component >= values.len() {
            return Ok(FieldValue::MissingComponent);
        }
        return resolve_path(values.as_ref(), component, remaining);
    }
    scalar_value(array, row)
}

pub(super) fn list_values(array: &dyn Array, row: usize) -> Result<ArrayRef, ChartError> {
    match array.data_type() {
        DataType::List(_) => Ok(array
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("List data type has ListArray storage")
            .value(row)),
        DataType::LargeList(_) => Ok(array
            .as_any()
            .downcast_ref::<LargeListArray>()
            .expect("LargeList data type has LargeListArray storage")
            .value(row)),
        DataType::ListView(_) => Ok(array
            .as_any()
            .downcast_ref::<ListViewArray>()
            .expect("ListView data type has ListViewArray storage")
            .value(row)),
        DataType::LargeListView(_) => Ok(array
            .as_any()
            .downcast_ref::<LargeListViewArray>()
            .expect("LargeListView data type has LargeListViewArray storage")
            .value(row)),
        DataType::FixedSizeList(_, _) => Ok(array
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .expect("FixedSizeList data type has FixedSizeListArray storage")
            .value(row)),
        data_type => Err(ChartError::new(
            "chart_component_invalid",
            "标量字段不能使用 List 分量路径",
        )
        .with_details(serde_json::json!({"observedType": data_type.to_string()}))),
    }
}

fn scalar_value(array: &dyn Array, row: usize) -> Result<FieldValue, ChartError> {
    macro_rules! integer {
        ($array:ty) => {
            if let Some(array) = array.as_any().downcast_ref::<$array>() {
                return Ok(FieldValue::Category(CategoryValue::Integer {
                    value: array.value(row).to_string(),
                }));
            }
        };
    }
    match array.data_type() {
        DataType::Utf8 => Ok(FieldValue::Category(CategoryValue::String {
            value: array
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8 data type has StringArray storage")
                .value(row)
                .into(),
        })),
        DataType::LargeUtf8 => Ok(FieldValue::Category(CategoryValue::String {
            value: array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .expect("LargeUtf8 data type has LargeStringArray storage")
                .value(row)
                .into(),
        })),
        DataType::Utf8View => Ok(FieldValue::Category(CategoryValue::String {
            value: array
                .as_any()
                .downcast_ref::<StringViewArray>()
                .expect("Utf8View data type has StringViewArray storage")
                .value(row)
                .into(),
        })),
        DataType::Boolean => Ok(FieldValue::Category(CategoryValue::Boolean {
            value: array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("Boolean data type has BooleanArray storage")
                .value(row),
        })),
        DataType::Int8 => {
            integer!(Int8Array);
            unreachable!()
        }
        DataType::Int16 => {
            integer!(Int16Array);
            unreachable!()
        }
        DataType::Int32 => {
            integer!(Int32Array);
            unreachable!()
        }
        DataType::Int64 => {
            integer!(Int64Array);
            unreachable!()
        }
        DataType::UInt8 => {
            integer!(UInt8Array);
            unreachable!()
        }
        DataType::UInt16 => {
            integer!(UInt16Array);
            unreachable!()
        }
        DataType::UInt32 => {
            integer!(UInt32Array);
            unreachable!()
        }
        DataType::UInt64 => {
            integer!(UInt64Array);
            unreachable!()
        }
        DataType::Float32 => finite_number(
            array
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("Float32 data type has Float32Array storage")
                .value(row) as f64,
        ),
        DataType::Float64 => finite_number(
            array
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("Float64 data type has Float64Array storage")
                .value(row),
        ),
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_)
        | DataType::FixedSizeList(_, _) => Err(ChartError::new(
            "chart_component_invalid",
            "List 字段必须选择具体分量",
        )
        .with_details(serde_json::json!({"observedType": array.data_type().to_string()}))),
        data_type => Err(incompatible(
            "字段物理类型不受图表支持",
            Some(data_type.to_string().as_str()),
        )),
    }
}

fn finite_number(value: f64) -> Result<FieldValue, ChartError> {
    if value.is_finite() {
        Ok(FieldValue::Number(value))
    } else {
        Ok(FieldValue::NonFinite)
    }
}

fn incompatible(message: impl Into<String>, observed_type: Option<&str>) -> ChartError {
    let error = ChartError::new("chart_column_incompatible", message);
    match observed_type {
        Some(observed_type) => {
            error.with_details(serde_json::json!({"observedType": observed_type}))
        }
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use arrow_array::builder::{
        FixedSizeListBuilder, Float64Builder, LargeListBuilder, LargeListViewBuilder, ListBuilder,
        ListViewBuilder,
    };
    use arrow_array::{BooleanArray, Float64Array, Int64Array, StringViewArray, UInt64Array};

    use super::*;
    use crate::application::charting::{CategoryValue, ChartFieldRef};

    fn field(path: Option<Vec<u16>>) -> ChartFieldRef {
        ChartFieldRef {
            column: "value".into(),
            path,
            label: None,
        }
    }

    #[test]
    fn scalar_values_preserve_category_identity_and_reject_unsafe_value_axes() {
        let strings = StringViewArray::from(vec![Some("bus-1"), None]);
        let booleans = BooleanArray::from(vec![Some(true)]);
        let signed = Int64Array::from(vec![Some(42), Some(i64::MAX)]);
        let unsigned = UInt64Array::from(vec![Some(u64::MAX)]);
        let numbers = Float64Array::from(vec![Some(1.5), Some(f64::NAN), None]);

        assert_eq!(
            resolve_field(&strings, 0, &field(None)).unwrap(),
            FieldValue::Category(CategoryValue::String {
                value: "bus-1".into()
            })
        );
        assert_eq!(
            resolve_field(&strings, 1, &field(None)).unwrap(),
            FieldValue::Missing
        );
        assert_eq!(
            resolve_field(&booleans, 0, &field(None)).unwrap(),
            FieldValue::Category(CategoryValue::Boolean { value: true })
        );
        assert_eq!(
            resolve_field(&signed, 0, &field(None))
                .unwrap()
                .as_number()
                .unwrap(),
            42.0
        );
        assert_eq!(
            resolve_field(&signed, 1, &field(None))
                .unwrap()
                .as_number()
                .unwrap_err()
                .code,
            "chart_column_incompatible"
        );
        assert_eq!(
            resolve_field(&unsigned, 0, &field(None)).unwrap(),
            FieldValue::Category(CategoryValue::Integer {
                value: u64::MAX.to_string()
            })
        );
        assert_eq!(
            resolve_field(&numbers, 0, &field(None)).unwrap(),
            FieldValue::Number(1.5)
        );
        assert_eq!(
            resolve_field(&numbers, 1, &field(None)).unwrap(),
            FieldValue::NonFinite
        );
        assert_eq!(
            resolve_field(&numbers, 2, &field(None)).unwrap(),
            FieldValue::Missing
        );
    }

    #[test]
    fn list_families_and_four_level_paths_resolve_exact_components() {
        let mut list = ListBuilder::new(Float64Builder::new());
        list.values().append_slice(&[1.0, 2.0, 3.0]);
        list.append(true);
        let list = list.finish();

        let mut large = LargeListBuilder::new(Float64Builder::new());
        large.values().append_slice(&[4.0, 5.0, 6.0]);
        large.append(true);
        let large = large.finish();

        let mut fixed = FixedSizeListBuilder::new(Float64Builder::new(), 3);
        fixed.values().append_slice(&[7.0, 8.0, 9.0]);
        fixed.append(true);
        let fixed = fixed.finish();

        let mut view = ListViewBuilder::new(Float64Builder::new());
        view.values().append_slice(&[10.0, 11.0, 12.0]);
        view.append(true);
        let view = view.finish();

        let mut large_view = LargeListViewBuilder::new(Float64Builder::new());
        large_view.values().append_slice(&[13.0, 14.0, 15.0]);
        large_view.append(true);
        let large_view = large_view.finish();

        assert_eq!(
            resolve_field(&list, 0, &field(Some(vec![2]))).unwrap(),
            FieldValue::Number(3.0)
        );
        assert_eq!(
            resolve_field(&large, 0, &field(Some(vec![1]))).unwrap(),
            FieldValue::Number(5.0)
        );
        assert_eq!(
            resolve_field(&fixed, 0, &field(Some(vec![0]))).unwrap(),
            FieldValue::Number(7.0)
        );
        assert_eq!(
            resolve_field(&view, 0, &field(Some(vec![2]))).unwrap(),
            FieldValue::Number(12.0)
        );
        assert_eq!(
            resolve_field(&large_view, 0, &field(Some(vec![1]))).unwrap(),
            FieldValue::Number(14.0)
        );

        let mut nested = ListBuilder::new(ListBuilder::new(ListBuilder::new(ListBuilder::new(
            Float64Builder::new(),
        ))));
        nested
            .values()
            .values()
            .values()
            .values()
            .append_value(21.0);
        nested.values().values().values().append(true);
        nested.values().values().append(true);
        nested.values().append(true);
        nested.append(true);
        let nested = nested.finish();
        assert_eq!(
            resolve_field(&nested, 0, &field(Some(vec![0, 0, 0, 0]))).unwrap(),
            FieldValue::Number(21.0)
        );
    }

    #[test]
    fn missing_null_and_non_finite_components_are_distinct() {
        let mut values = LargeListBuilder::new(Float64Builder::new());
        values.values().append_slice(&[1.0, 2.0, 3.0]);
        values.append(true);
        values.values().append_slice(&[4.0, 5.0]);
        values.append(true);
        values.append(false);
        values.values().append_value(f64::INFINITY);
        values.append(true);
        let values = values.finish();

        assert_eq!(
            resolve_field(&values, 0, &field(Some(vec![2]))).unwrap(),
            FieldValue::Number(3.0)
        );
        assert_eq!(
            resolve_field(&values, 1, &field(Some(vec![2]))).unwrap(),
            FieldValue::MissingComponent
        );
        assert_eq!(
            resolve_field(&values, 2, &field(Some(vec![0]))).unwrap(),
            FieldValue::Missing
        );
        assert_eq!(
            resolve_field(&values, 3, &field(Some(vec![0]))).unwrap(),
            FieldValue::NonFinite
        );
    }
}
