use std::io::Read;
use std::path::Path;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use calamine::{Data, Reader, SheetType, SheetVisible, open_workbook_auto};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::properties::WriterProperties;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSchema {
    pub name: String,
    pub dtype: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabularFormat {
    Csv,
    Xls,
    Xlsx,
    Parquet,
}

#[derive(Debug)]
pub struct TabularError {
    code: &'static str,
    message: String,
}

impl TabularError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for TabularError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for TabularError {}

pub struct TabularInspector;

#[derive(Debug, Clone)]
pub struct NormalizedSheet {
    pub sheet_name: String,
    pub path: std::path::PathBuf,
    pub schema: TableSchema,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcelSafetyInspection {
    pub visible_nonempty_sheets: Vec<String>,
    pub executed_formulas: bool,
    pub executed_macros: bool,
    pub followed_external_links: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaCell {
    pub position: String,
    pub required: bool,
    pub cached: Option<String>,
}

pub fn validate_formula_cache(cells: &[FormulaCell]) -> Result<(), TabularError> {
    let missing = cells
        .iter()
        .filter(|cell| cell.required && cell.cached.is_none())
        .map(|cell| cell.position.clone())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(TabularError::new(
            "excel_formula_value_missing",
            format!("必填公式缺少缓存值: {}", missing.join(", ")),
        ))
    }
}

pub fn inspect_excel_safely(path: &Path) -> Result<ExcelSafetyInspection, TabularError> {
    validate_xlsx_formula_caches(path)?;
    let mut workbook = open_workbook_auto(path)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let visible = workbook
        .sheets_metadata()
        .iter()
        .filter(|sheet| sheet.visible == SheetVisible::Visible && sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect::<Vec<_>>();
    let mut visible_nonempty_sheets = Vec::new();
    for name in visible {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
        if range
            .rows()
            .any(|row| row.iter().any(|cell| !matches!(cell, Data::Empty)))
        {
            visible_nonempty_sheets.push(name);
        }
    }
    Ok(ExcelSafetyInspection {
        visible_nonempty_sheets,
        executed_formulas: false,
        executed_macros: false,
        followed_external_links: false,
    })
}

impl TabularInspector {
    pub fn inspect(path: &Path, format: TabularFormat) -> Result<Vec<TableSchema>, TabularError> {
        match format {
            TabularFormat::Csv => inspect_csv(path),
            TabularFormat::Xls | TabularFormat::Xlsx => inspect_excel(path),
            TabularFormat::Parquet => inspect_parquet(path),
        }
    }
}

pub fn normalize_excel_to_parquet(
    source: &Path,
    output_dir: &Path,
) -> Result<Vec<NormalizedSheet>, TabularError> {
    validate_xlsx_formula_caches(source)?;
    std::fs::create_dir_all(output_dir)
        .map_err(|error| TabularError::new("tabular_normalize_failed", error.to_string()))?;
    let mut workbook = open_workbook_auto(source)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let mut outputs = Vec::new();
    let visible = workbook
        .sheets_metadata()
        .iter()
        .filter(|sheet| sheet.visible == SheetVisible::Visible && sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect::<Vec<_>>();
    for (sheet_index, sheet_name) in visible.into_iter().enumerate() {
        let range = workbook
            .worksheet_range(&sheet_name)
            .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
        let mut rows = range
            .rows()
            .filter(|row| row.iter().any(|cell| !matches!(cell, Data::Empty)));
        let Some(header) = rows.next() else { continue };
        let headers: Vec<String> = header
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let value = cell.to_string();
                if value.trim().is_empty() {
                    format!("column_{}", index + 1)
                } else {
                    value
                }
            })
            .collect();
        if headers.is_empty() {
            continue;
        }
        let data: Vec<Vec<String>> = rows
            .map(|row| {
                (0..headers.len())
                    .map(|index| row.get(index).map(ToString::to_string).unwrap_or_default())
                    .collect()
            })
            .collect();
        let schema = Arc::new(Schema::new(
            headers
                .iter()
                .map(|name| Field::new(name, DataType::Utf8, true))
                .collect::<Vec<_>>(),
        ));
        let arrays: Vec<ArrayRef> = (0..headers.len())
            .map(|column| {
                Arc::new(StringArray::from(
                    data.iter()
                        .map(|row| row[column].as_str())
                        .collect::<Vec<_>>(),
                )) as ArrayRef
            })
            .collect();
        let batch = RecordBatch::try_new(schema.clone(), arrays)
            .map_err(|error| TabularError::new("tabular_normalize_failed", error.to_string()))?;
        let path = output_dir.join(format!("sheet-{sheet_index}.parquet"));
        let file = std::fs::File::create(&path)
            .map_err(|error| TabularError::new("tabular_normalize_failed", error.to_string()))?;
        let properties = WriterProperties::builder()
            .set_compression(parquet::basic::Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(properties))
            .map_err(|error| TabularError::new("tabular_normalize_failed", error.to_string()))?;
        writer
            .write(&batch)
            .and_then(|_| writer.close().map(|_| ()))
            .map_err(|error| TabularError::new("tabular_normalize_failed", error.to_string()))?;
        outputs.push(NormalizedSheet {
            sheet_name: sheet_name.clone(),
            path,
            schema: TableSchema {
                name: sheet_name,
                columns: headers
                    .into_iter()
                    .map(|name| ColumnSchema {
                        name,
                        dtype: "string".into(),
                    })
                    .collect(),
            },
        });
    }
    Ok(outputs)
}

fn validate_xlsx_formula_caches(path: &Path) -> Result<(), TabularError> {
    let signature = std::fs::read(path)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    if !signature.starts_with(b"PK") {
        return Ok(());
    }
    let file = std::fs::File::open(path)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let mut missing = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
        let entry_name = entry
            .name()
            .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?
            .into_owned();
        if !entry_name.starts_with("xl/worksheets/") || !entry_name.ends_with(".xml") {
            continue;
        }
        let sheet = entry_name;
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
        for cell in xml.split("</c>") {
            if cell.contains("<f") && !cell.contains("<v>") {
                let position = cell
                    .rsplit("<c ")
                    .next()
                    .and_then(|start| start.split("r=\"").nth(1))
                    .and_then(|rest| rest.split('"').next())
                    .unwrap_or("?");
                missing.push(format!("{sheet}:{position}"));
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(TabularError::new(
            "excel_formula_value_missing",
            format!("公式缺少缓存值: {}", missing.join(", ")),
        ))
    }
}

/// 把 CSV 确定性地规范化为 UTF-8 Parquet，供 tensoreval 命名输入使用。
pub fn normalize_csv_to_parquet(source: &Path, output: &Path) -> Result<TableSchema, TabularError> {
    normalize_csv_with_settings(source, output, b',', 0)
}

pub fn normalize_csv_with_settings(
    source: &Path,
    output: &Path,
    delimiter: u8,
    header_row: u32,
) -> Result<TableSchema, TabularError> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .has_headers(false)
        .from_path(source)
        .map_err(|e| TabularError::new("tabular_read_failed", e.to_string()))?;
    let mut records = reader.records();
    let headers = records
        .nth(header_row as usize)
        .transpose()
        .map_err(|e| TabularError::new("tabular_header_invalid", e.to_string()))?
        .ok_or_else(|| TabularError::new("tabular_header_invalid", "表头行超出文件范围"))?;
    if headers.is_empty() {
        return Err(TabularError::new("tabular_empty", "CSV 没有列"));
    }
    let rows = records
        .map(|r| r.map(|r| r.iter().map(str::to_string).collect::<Vec<_>>()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TabularError::new("tabular_read_failed", e.to_string()))?;
    let schema = Arc::new(Schema::new(
        headers
            .iter()
            .map(|h| Field::new(h, DataType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let arrays = (0..headers.len())
        .map(|col| {
            Arc::new(StringArray::from(
                rows.iter()
                    .map(|r| r.get(col).map(String::as_str))
                    .collect::<Vec<_>>(),
            )) as ArrayRef
        })
        .collect();
    let batch = RecordBatch::try_new(schema.clone(), arrays)
        .map_err(|e| TabularError::new("tabular_normalize_failed", e.to_string()))?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TabularError::new("tabular_normalize_failed", e.to_string()))?;
    }
    let file = std::fs::File::create(output)
        .map_err(|e| TabularError::new("tabular_normalize_failed", e.to_string()))?;
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props))
        .map_err(|e| TabularError::new("tabular_normalize_failed", e.to_string()))?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|e| TabularError::new("tabular_normalize_failed", e.to_string()))?;
    Ok(TableSchema {
        name: source
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("table")
            .into(),
        columns: headers
            .iter()
            .map(|h| ColumnSchema {
                name: h.into(),
                dtype: "string".into(),
            })
            .collect(),
    })
}

fn inspect_csv(path: &Path) -> Result<Vec<TableSchema>, TabularError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let headers = reader
        .headers()
        .map_err(|error| TabularError::new("tabular_header_invalid", error.to_string()))?
        .clone();
    if headers.is_empty() {
        return Ok(Vec::new());
    }
    let mut kinds = vec![Observed::Unknown; headers.len()];
    for row in reader.records().take(100) {
        let row =
            row.map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
        for (index, value) in row.iter().enumerate().take(kinds.len()) {
            kinds[index] = kinds[index].merge(observe_text(value));
        }
    }
    Ok(vec![TableSchema {
        name: path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("table")
            .to_string(),
        columns: headers
            .iter()
            .zip(kinds)
            .map(|(name, kind)| ColumnSchema {
                name: name.to_string(),
                dtype: kind.name().to_string(),
            })
            .collect(),
    }])
}

fn inspect_excel(path: &Path) -> Result<Vec<TableSchema>, TabularError> {
    let mut workbook = open_workbook_auto(path)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let names = workbook
        .sheets_metadata()
        .iter()
        .filter(|sheet| sheet.visible == SheetVisible::Visible && sheet.typ == SheetType::WorkSheet)
        .map(|sheet| sheet.name.clone())
        .collect::<Vec<_>>();
    let mut tables = Vec::new();
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
        let mut rows = range
            .rows()
            .filter(|row| row.iter().any(|cell| !matches!(cell, Data::Empty)));
        let Some(header) = rows.next() else {
            continue;
        };
        let headers: Vec<String> = header.iter().map(ToString::to_string).collect();
        if headers.iter().all(|value| value.trim().is_empty()) {
            continue;
        }
        let mut kinds = vec![Observed::Unknown; headers.len()];
        for row in rows.take(100) {
            for (index, cell) in row.iter().enumerate().take(kinds.len()) {
                kinds[index] = kinds[index].merge(observe_cell(cell));
            }
        }
        tables.push(TableSchema {
            name,
            columns: headers
                .into_iter()
                .zip(kinds)
                .map(|(name, kind)| ColumnSchema {
                    name,
                    dtype: kind.name().to_string(),
                })
                .collect(),
        });
    }
    Ok(tables)
}

fn inspect_parquet(path: &Path) -> Result<Vec<TableSchema>, TabularError> {
    let file = std::fs::File::open(path)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| TabularError::new("tabular_read_failed", error.to_string()))?;
    let columns = builder
        .schema()
        .fields()
        .iter()
        .map(|field| ColumnSchema {
            name: field.name().to_string(),
            dtype: field.data_type().to_string(),
        })
        .collect();
    Ok(vec![TableSchema {
        name: path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("table")
            .to_string(),
        columns,
    }])
}

#[derive(Clone, Copy)]
enum Observed {
    Unknown,
    Bool,
    Int,
    Float,
    String,
}

impl Observed {
    fn merge(self, next: Self) -> Self {
        use Observed::*;
        match (self, next) {
            (Unknown, value) | (value, Unknown) => value,
            (left, right) if left as u8 == right as u8 => left,
            (Int, Float) | (Float, Int) => Float,
            _ => String,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Bool => "bool",
            Self::Int => "int64",
            Self::Float => "float64",
            Self::String => "string",
        }
    }
}

fn observe_text(value: &str) -> Observed {
    let value = value.trim();
    if value.is_empty() {
        Observed::Unknown
    } else if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") {
        Observed::Bool
    } else if value.parse::<i64>().is_ok() {
        Observed::Int
    } else if value.parse::<f64>().is_ok() {
        Observed::Float
    } else {
        Observed::String
    }
}

fn observe_cell(cell: &Data) -> Observed {
    match cell {
        Data::Empty => Observed::Unknown,
        Data::Bool(_) => Observed::Bool,
        Data::Int(_) => Observed::Int,
        Data::Float(_) | Data::DateTime(_) | Data::DateTimeIso(_) | Data::DurationIso(_) => {
            Observed::Float
        }
        Data::String(_) | Data::Error(_) => Observed::String,
    }
}
