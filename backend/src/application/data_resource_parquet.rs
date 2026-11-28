use std::collections::BTreeSet;
use std::sync::Arc;

use arrow_array::types::Float64Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, RecordBatch,
    StringViewArray, UInt32Array, UInt64Array,
};
use arrow_buffer::{OffsetBuffer, ScalarBuffer};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde_json::Value;

use crate::catalog::{DataFrameSchemaContract, DataTypeContract};

use super::{DataResourceError, DataResourceService};

pub(crate) fn struct_parquet(
    contract: &DataFrameSchemaContract,
    values: &serde_json::Map<String, Value>,
) -> Result<Vec<u8>, DataResourceError> {
    let known = contract
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if values.keys().any(|name| !known.contains(name.as_str())) {
        return Err(DataResourceService::invalid(
            "structured_values_invalid",
            "values 包含 schema 未声明字段",
        ));
    }
    let mut fields = Vec::new();
    let mut arrays = Vec::<ArrayRef>::new();
    for field in &contract.fields {
        let value = values.get(&field.name);
        if field.required && value.is_none_or(Value::is_null) {
            return Err(DataResourceService::invalid(
                "structured_values_invalid",
                &format!("缺少必填字段 {}", field.name),
            ));
        }
        let is_null = value.is_none_or(Value::is_null);
        let is_list = matches!(&field.dtype, DataTypeContract::Constructor { .. });
        if !field.required && field.nullable && is_null && is_list {
            continue;
        }
        let (dtype, array) = struct_array(&field.dtype, value, field.nullable, &field.name)?;
        fields.push(Field::new(&field.name, dtype, field.nullable));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays).map_err(|error| {
        DataResourceService::invalid("structured_values_invalid", &error.to_string())
    })?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut output = Vec::new();
    let mut writer =
        ArrowWriter::try_new(&mut output, schema, Some(properties)).map_err(|error| {
            DataResourceService::invalid("structured_values_invalid", &error.to_string())
        })?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|error| {
            DataResourceService::invalid("structured_values_invalid", &error.to_string())
        })?;
    Ok(output)
}
fn struct_array(
    dtype: &DataTypeContract,
    value: Option<&Value>,
    nullable: bool,
    name: &str,
) -> Result<(DataType, ArrayRef), DataResourceError> {
    let null = value.is_none_or(Value::is_null);
    if null && !nullable {
        return Err(DataResourceService::invalid(
            "structured_values_invalid",
            &format!("字段 {name} 不能为空"),
        ));
    }
    match dtype {
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "str" || kind.contains("string") =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_str().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是字符串"),
                    )
                })?)
            };
            Ok((DataType::Utf8View, Arc::new(StringViewArray::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "bool" =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_bool().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是布尔值"),
                    )
                })?)
            };
            Ok((DataType::Boolean, Arc::new(BooleanArray::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "f64" || kind.contains("float") =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_f64().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是数值"),
                    )
                })?)
            };
            Ok((DataType::Float64, Arc::new(Float64Array::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if matches_u64_kind(kind) =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_u64().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是非负整数"),
                    )
                })?)
            };
            Ok((DataType::UInt64, Arc::new(UInt64Array::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if matches_uint_kind(kind) =>
        {
            let item = if null {
                None
            } else {
                Some(
                    u32::try_from(value.unwrap().as_u64().ok_or_else(|| {
                        DataResourceService::invalid(
                            "structured_values_invalid",
                            &format!("字段 {name} 必须是非负整数"),
                        )
                    })?)
                    .map_err(|_| {
                        DataResourceService::invalid(
                            "structured_values_invalid",
                            &format!("字段 {name} 超出 u32 范围"),
                        )
                    })?,
                )
            };
            Ok((DataType::UInt32, Arc::new(UInt32Array::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if matches_int_kind(kind) =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_i64().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是整数"),
                    )
                })?)
            };
            Ok((DataType::Int64, Arc::new(Int64Array::from(vec![item]))))
        }
        DataTypeContract::Constructor {
            name: kind,
            element,
        } if kind == "list"
            && matches!(element.as_ref(),DataTypeContract::Primitive{name} if name=="f64") =>
        {
            let items = if null {
                None
            } else {
                Some(
                    value
                        .unwrap()
                        .as_array()
                        .ok_or_else(|| {
                            DataResourceService::invalid(
                                "structured_values_invalid",
                                &format!("字段 {name} 必须是数组"),
                            )
                        })?
                        .iter()
                        .map(|item| {
                            item.as_f64().ok_or_else(|| {
                                DataResourceService::invalid(
                                    "structured_values_invalid",
                                    &format!("字段 {name} 包含非数值"),
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            };
            let array = ListArray::from_iter_primitive::<Float64Type, _, _>(vec![
                items.map(|row| row.into_iter().map(Some).collect::<Vec<_>>()),
            ]);
            Ok((array.data_type().clone(), Arc::new(array)))
        }
        DataTypeContract::Constructor {
            name: kind,
            element,
        } if kind == "list"
            && matches!(element.as_ref(),DataTypeContract::Primitive{name} if name=="str") =>
        {
            let items = if null {
                None
            } else {
                Some(
                    value
                        .unwrap()
                        .as_array()
                        .ok_or_else(|| {
                            DataResourceService::invalid(
                                "structured_values_invalid",
                                &format!("字段 {name} 必须是数组"),
                            )
                        })?
                        .iter()
                        .map(|item| {
                            item.as_str().ok_or_else(|| {
                                DataResourceService::invalid(
                                    "structured_values_invalid",
                                    &format!("字段 {name} 包含非字符串项"),
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            };
            let inner_values = items.clone().unwrap_or_default();
            let inner = StringViewArray::from(
                inner_values.iter().map(|s| Some(*s)).collect::<Vec<_>>(),
            );
            let inner_len = inner.len();
            let scalar = ScalarBuffer::<i32>::from(vec![0i32, inner_len as i32]);
            let offsets = OffsetBuffer::<i32>::new(scalar);
            let list_field = Arc::new(Field::new("item", DataType::Utf8View, true));
            let array = ListArray::new(
                list_field,
                offsets,
                Arc::new(inner),
                if items.is_none() { Some(arrow_buffer::NullBuffer::from(vec![false])) } else { None },
            );
            Ok((array.data_type().clone(), Arc::new(array)))
        }
        _ => Err(DataResourceService::invalid(
            "structured_values_invalid",
            &format!("字段 {name} 的 dtype 暂不支持"),
        )),
    }
}
pub(super) fn matches_int_kind(kind: &str) -> bool {
    matches!(kind, "i8" | "i16" | "i32" | "i64")
        || kind.starts_with("i8_")
        || kind.starts_with("i16_")
        || kind.starts_with("i32_")
        || kind.starts_with("i64_")
        || kind.contains("int")
}

pub(super) fn matches_uint_kind(kind: &str) -> bool {
    matches!(kind, "u8" | "u16" | "u32")
        || kind.starts_with("u8_")
        || kind.starts_with("u16_")
        || kind.starts_with("u32_")
}

fn matches_u64_kind(kind: &str) -> bool {
    kind == "u64" || kind.starts_with("u64_")
}

pub(super) fn dtype_label(dtype: &DataTypeContract) -> String {
    match dtype {
        DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => name.clone(),
        DataTypeContract::Constructor { name, element } => {
            format!("{}<{}>", name, dtype_label(element))
        }
    }
}