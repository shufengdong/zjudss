//! 托管工作流结果的逻辑 schema 与非空值合同校验。

use std::path::Path;

use crate::catalog::{DataTypeContract, SchemaFieldContract};
use crate::storage::{ParquetInspector, ParquetMetadata};

pub(super) fn validate_schema(
    metadata: &ParquetMetadata,
    expected: &[SchemaFieldContract],
) -> Result<(), String> {
    if metadata.columns.len() != expected.len() {
        return Err(format!(
            "结果列数不一致: expected {}, actual {}",
            expected.len(),
            metadata.columns.len()
        ));
    }
    for (actual, expected) in metadata.columns.iter().zip(expected) {
        let expected_dtype = canonical_arrow(&expected.dtype);
        let expected_unit =
            (expected.unit.0 != "not_applicable").then_some(expected.unit.0.as_str());
        let unit_mismatch = actual
            .unit
            .as_deref()
            .is_some_and(|actual| Some(actual) != expected_unit);
        if actual.name != expected.name
            || !dtype_matches(&actual.dtype, &expected.dtype)
            || unit_mismatch
        {
            return Err(format!(
                "结果列 {} 不符合合同: expected {}/{}/{:?}, actual {}/{}/{:?}",
                expected.name,
                expected_dtype,
                expected.nullable,
                expected_unit,
                actual.dtype,
                actual.nullable,
                actual.unit
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_non_null_values(
    path: &Path,
    expected: &[SchemaFieldContract],
) -> Result<(), String> {
    let columns = expected
        .iter()
        .filter(|field| !field.nullable)
        .map(|field| field.name.clone())
        .collect::<Vec<_>>();
    match ParquetInspector::first_column_with_nulls(path, &columns) {
        Ok(Some(name)) => Err(format!("结果列 {name} 含 null，不符合非空合同")),
        Ok(None) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn canonical_arrow(dtype: &DataTypeContract) -> String {
    match dtype {
        DataTypeContract::Primitive { name } => match name.as_str() {
            "str" => "Utf8".into(),
            "bool" => "Boolean".into(),
            "i32" => "Int32".into(),
            "i64" => "Int64".into(),
            "u32" => "UInt32".into(),
            "u64" => "UInt64".into(),
            "f32" => "Float32".into(),
            "f64" => "Float64".into(),
            other => other.into(),
        },
        DataTypeContract::Semantic { name } => match name.as_str() {
            "u32_semantics" => "UInt32".into(),
            other => other.into(),
        },
        DataTypeContract::Constructor { name, element } => {
            format!("{}<{}>", title_case(name), canonical_arrow(element))
        }
    }
}

fn dtype_matches(actual: &str, expected: &DataTypeContract) -> bool {
    match expected {
        DataTypeContract::Primitive { name } if name == "str" => {
            matches!(actual, "Utf8" | "LargeUtf8" | "Utf8View")
        }
        DataTypeContract::Constructor { name, element }
            if matches!(name.as_str(), "list" | "matrix") =>
        {
            let element = canonical_arrow(element);
            ["List", "LargeList", "ListView", "LargeListView"]
                .iter()
                .any(|kind| {
                    actual == format!("{kind}({element})")
                        || actual.starts_with(&format!("{kind}({element},"))
                        || actual.starts_with(&format!("{kind}(non-null {element}"))
                })
        }
        _ => actual == canonical_arrow(expected),
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{Float64Array, RecordBatch};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;

    use super::{dtype_matches, validate_non_null_values, validate_schema};
    use crate::catalog::{DataTypeContract, SchemaFieldContract, UnitContract};
    use crate::storage::{ParquetColumn, ParquetMetadata};

    fn constructor(name: &str, element: &str) -> DataTypeContract {
        DataTypeContract::Constructor {
            name: name.into(),
            element: Box::new(DataTypeContract::Primitive {
                name: element.into(),
            }),
        }
    }

    #[test]
    fn accepts_arrow_list_display_for_list_and_matrix_contracts() {
        let arrow = "List(Float64, field: 'element')";
        assert!(dtype_matches(arrow, &constructor("list", "f64")));
        assert!(dtype_matches(arrow, &constructor("matrix", "f64")));
    }

    #[test]
    fn accepts_large_list_and_rejects_wrong_element_types() {
        let expected = constructor("list", "f64");
        assert!(dtype_matches(
            "LargeList(Float64, field: 'element')",
            &expected
        ));
        assert!(!dtype_matches("List(Utf8, field: 'element')", &expected));
    }

    #[test]
    fn accepts_arrow_string_storage_variants_for_string_contracts() {
        let expected = DataTypeContract::Primitive { name: "str".into() };
        assert!(dtype_matches("Utf8", &expected));
        assert!(dtype_matches("LargeUtf8", &expected));
        assert!(dtype_matches("Utf8View", &expected));
        assert!(!dtype_matches("BinaryView", &expected));
    }

    #[test]
    fn logical_schema_accepts_physical_nullability_and_missing_unit_metadata() {
        let metadata = ParquetMetadata {
            columns: vec![ParquetColumn {
                name: "value".into(),
                dtype: "LargeList(Float64, field: 'element')".into(),
                nullable: true,
                unit: None,
            }],
            row_count: 1,
            size_bytes: 1,
            snappy: true,
        };
        let expected = vec![SchemaFieldContract {
            name: "value".into(),
            dtype: constructor("list", "f64"),
            required: true,
            nullable: false,
            unit: UnitContract("V".into()),
            card: None,
        }];
        assert!(validate_schema(&metadata, &expected).is_ok());

        let mut conflicting = metadata;
        conflicting.columns[0].unit = Some("A".into());
        assert!(validate_schema(&conflicting, &expected).is_err());
    }

    #[test]
    fn rejects_nulls_in_non_nullable_result_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("result.parquet");
        let schema = Arc::new(Schema::new(vec![Field::new(
            "value",
            DataType::Float64,
            true,
        )]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![Arc::new(Float64Array::from(vec![Some(1.0), None]))],
        )
        .unwrap();
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(
            std::fs::File::create(&path).unwrap(),
            schema,
            Some(properties),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        let expected = vec![SchemaFieldContract {
            name: "value".into(),
            dtype: DataTypeContract::Primitive { name: "f64".into() },
            required: true,
            nullable: false,
            unit: UnitContract("not_applicable".into()),
            card: None,
        }];

        let error = validate_non_null_values(&path, &expected).unwrap_err();
        assert!(error.contains("结果列 value 含 null"));
    }
}
