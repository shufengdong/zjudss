use std::collections::BTreeSet;
use std::sync::Arc;

use arrow_array::types::Float64Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, RecordBatch, StringArray,
    UInt32Array,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde_json::Value;

use crate::catalog::{DataFrameSchemaContract, DataTypeContract};

use super::{DataResourceError, DataResourceService};

pub(super) fn struct_parquet(
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
            Ok((DataType::Utf8, Arc::new(StringArray::from(vec![item]))))
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
            if kind == "u32" || kind.contains("u32") =>
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
            if kind == "i64" || kind.contains("int") =>
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
        _ => Err(DataResourceService::invalid(
            "structured_values_invalid",
            &format!("字段 {name} 的 dtype 暂不支持"),
        )),
    }
}
pub(super) fn dtype_label(dtype: &DataTypeContract) -> String {
    match dtype {
        DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => name.clone(),
        DataTypeContract::Constructor { name, element } => {
            format!("{}<{}>", name, dtype_label(element))
        }
    }
}
