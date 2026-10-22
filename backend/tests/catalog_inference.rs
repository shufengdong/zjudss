use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use backend::catalog::{SchemaCatalog, infer_table, infer_tables};
use backend::ingestion::{ColumnSchema, TableSchema, TabularFormat, TabularInspector};
use parquet::arrow::ArrowWriter;
use serde_json::json;

fn catalog() -> SchemaCatalog {
    SchemaCatalog::from_value("test-v1", &json!({
        "schema_version":2,
        "frames":{
            "bus":{"required":{"id":"str","terminals":"list<str>"},"optional":{"grounded":"list<str>"}},
            "load":{"required":{"name":"str","bus":"str","p":"f64"},"optional":{"model":"str"}},
            "generator":{"required":{"name":"str","bus":"str","p":"f64"},"optional":{"dispatch":"str"}}
        }
    })).unwrap()
}

fn table(name: &str, columns: &[(&str, &str)]) -> TableSchema {
    TableSchema {
        name: name.into(),
        columns: columns
            .iter()
            .map(|(name, dtype)| ColumnSchema {
                name: (*name).into(),
                dtype: (*dtype).into(),
            })
            .collect(),
    }
}

#[test]
fn inference_uses_normalized_headers_and_reports_dtype_conflicts() {
    let evidence = infer_table(
        &table(
            "misleading-bus-name",
            &[
                ("\u{feff} Name ", "string"),
                ("BUS", "string"),
                ("p", "string"),
                ("model", "string"),
            ],
        ),
        &catalog(),
    );
    assert_eq!(evidence.table_kind.as_deref(), Some("load"));
    assert_eq!(evidence.resolution, "inferred");
    assert!(
        evidence
            .type_conflicts
            .iter()
            .any(|item| item.contains("expected f64"))
    );
}

#[test]
fn unknown_similar_and_duplicate_tables_remain_pending() {
    let unknown = infer_table(&table("load.csv", &[("mystery", "string")]), &catalog());
    assert!(unknown.table_kind.is_none());
    assert_eq!(unknown.resolution, "pending");

    let similar = infer_table(
        &table(
            "anything",
            &[("name", "string"), ("bus", "string"), ("p", "float64")],
        ),
        &catalog(),
    );
    assert!(
        similar.table_kind.is_none(),
        "load/generator tie must not auto-bind"
    );
    assert_eq!(similar.resolution, "pending");

    let schemas = vec![
        table(
            "one",
            &[
                ("name", "string"),
                ("bus", "string"),
                ("p", "float64"),
                ("model", "string"),
            ],
        ),
        table(
            "two",
            &[
                ("name", "string"),
                ("bus", "string"),
                ("p", "float64"),
                ("model", "string"),
            ],
        ),
    ];
    let duplicated = infer_tables(&schemas, &catalog());
    assert!(
        duplicated
            .iter()
            .all(|item| item.table_kind.as_deref() == Some("load") && item.resolution == "pending")
    );
}

#[test]
fn each_catalog_schema_has_a_positive_header_example() {
    let catalog = catalog();
    for frame in &catalog.frames {
        let columns: Vec<ColumnSchema> = frame
            .required
            .iter()
            .chain(frame.optional.iter())
            .map(|(name, dtype)| ColumnSchema {
                name: name.clone(),
                dtype: dtype.clone(),
            })
            .collect();
        let evidence = infer_table(
            &TableSchema {
                name: "ignored".into(),
                columns,
            },
            &catalog,
        );
        assert_eq!(
            evidence.table_kind.as_deref(),
            Some(frame.kind.as_str()),
            "{} must have a positive example",
            frame.kind
        );
    }
}

#[test]
fn csv_and_excel_inspection_reads_headers_samples_and_skips_empty_sheets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inference");
    let csv = TabularInspector::inspect(&root.join("load.csv"), TabularFormat::Csv).unwrap();
    assert_eq!(csv.len(), 1);
    assert_eq!(csv[0].columns[0].name, " Name ");
    assert_eq!(
        infer_table(&csv[0], &catalog()).table_kind.as_deref(),
        Some("load")
    );

    let excel = TabularInspector::inspect(&root.join("multi.xlsx"), TabularFormat::Xlsx).unwrap();
    assert_eq!(excel.len(), 1);
    assert_eq!(excel[0].name, "Loads");
    assert_eq!(
        infer_table(&excel[0], &catalog()).table_kind.as_deref(),
        Some("load")
    );
}

#[test]
fn parquet_inspection_reads_schema_from_footer_without_scanning_rows() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("misleading-name.parquet");
    let schema = Arc::new(Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("bus", DataType::Utf8, false),
        Field::new("p", DataType::Float64, false),
        Field::new("model", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["L1"])) as ArrayRef,
            Arc::new(StringArray::from(vec!["B1"])) as ArrayRef,
            Arc::new(Float64Array::from(vec![100.0])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("pq")])) as ArrayRef,
        ],
    )
    .unwrap();
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();

    let tables = TabularInspector::inspect(&path, TabularFormat::Parquet).unwrap();
    assert_eq!(tables[0].columns.len(), 4);
    assert_eq!(
        infer_table(&tables[0], &catalog()).table_kind.as_deref(),
        Some("load")
    );
}

#[test]
fn malformed_catalog_error_is_stable() {
    let error = SchemaCatalog::from_value("v", &json!({"frames":{}})).unwrap_err();
    assert_eq!(error.code(), "catalog_invalid");
}
