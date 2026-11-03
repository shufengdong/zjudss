use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{Array, Float64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use sha2::{Digest, Sha256};

use super::{
    BarSeries, BoxplotOutlier, BoxplotSeries, CategoryValue, ChartError, ChartModel, CompiledChart,
    HeatmapCell, HistogramBin, LinePoint, LineSeries, LineXValue, PlotData, ScatterSeries,
};
use crate::storage::{ArtifactStore, StageLimits, StagedArtifact};

#[derive(Debug)]
pub struct MaterializedPlot {
    pub staged: StagedArtifact,
    pub schema_json: String,
    pub schema_hash: String,
    pub content_hash: String,
    pub row_count: u64,
    pub point_count: u64,
    pub size_bytes: u64,
}

pub struct PlotDataCodec;

impl PlotDataCodec {
    pub fn write_snappy(
        store: &ArtifactStore,
        compiled: &CompiledChart,
    ) -> Result<MaterializedPlot, ChartError> {
        let bytes = Self::encode_snappy(compiled)?;
        let staged = store
            .stage(Cursor::new(bytes), StageLimits::default())
            .map_err(|error| materialization(error.to_string()))?;
        let schema_json = serde_json::to_string(&compiled.plot_schema)
            .map_err(|error| materialization(error.to_string()))?;
        let schema_hash = hex::encode(Sha256::digest(schema_json.as_bytes()));
        let content_hash = staged.sha256.clone();
        let size_bytes = staged.size_bytes;
        Ok(MaterializedPlot {
            staged,
            schema_json,
            schema_hash,
            content_hash,
            row_count: plot_row_count(&compiled.plot_data) as u64,
            point_count: compiled.diagnostics.output_points,
            size_bytes,
        })
    }

    pub fn read_model(file: File, frozen: &ChartModel) -> Result<ChartModel, ChartError> {
        let builder = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|error| materialization(error.to_string()))?;
        validate_columns(builder.schema().as_ref(), expected_columns(frozen))?;
        let batches = builder
            .build()
            .map_err(|error| materialization(error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| materialization(error.to_string()))?;
        decode_model(frozen.clone(), &batches)
    }

    pub(crate) fn encode_snappy(compiled: &CompiledChart) -> Result<Vec<u8>, ChartError> {
        let batch = plot_batch(&compiled.plot_data)?;
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut output = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut output, batch.schema(), Some(properties))
            .map_err(|error| materialization(error.to_string()))?;
        writer
            .write(&batch)
            .map_err(|error| materialization(error.to_string()))?;
        writer
            .close()
            .map_err(|error| materialization(error.to_string()))?;
        Ok(output)
    }
}

fn plot_batch(plot_data: &PlotData) -> Result<RecordBatch, ChartError> {
    match plot_data {
        PlotData::Line(rows) => {
            let schema = schema(vec![
                field("ordinal", DataType::UInt64, false),
                field("x_kind", DataType::Utf8, false),
                field("x_category_json", DataType::Utf8, true),
                field("x_value", DataType::Float64, true),
                field("series", DataType::Utf8, false),
                field("value", DataType::Float64, false),
            ]);
            record_batch(
                schema,
                vec![
                    Arc::new(UInt64Array::from_iter_values(
                        rows.iter().map(|row| row.ordinal),
                    )),
                    Arc::new(StringArray::from_iter_values(rows.iter().map(|row| {
                        if matches!(row.x, LineXValue::Category(_)) {
                            "category"
                        } else {
                            "value"
                        }
                    }))),
                    Arc::new(StringArray::from(
                        rows.iter()
                            .map(|row| match &row.x {
                                LineXValue::Category(value) => Some(category_json(value)),
                                LineXValue::Number(_) => None,
                            })
                            .collect::<Vec<_>>(),
                    )),
                    Arc::new(Float64Array::from(
                        rows.iter()
                            .map(|row| match row.x {
                                LineXValue::Category(_) => None,
                                LineXValue::Number(value) => Some(value),
                            })
                            .collect::<Vec<_>>(),
                    )),
                    Arc::new(StringArray::from_iter_values(
                        rows.iter().map(|row| row.series.as_str()),
                    )),
                    Arc::new(Float64Array::from_iter_values(
                        rows.iter().map(|row| row.value),
                    )),
                ],
            )
        }
        PlotData::Bar(rows) => record_batch(
            schema(vec![
                field("category_index", DataType::UInt64, false),
                field("category_json", DataType::Utf8, false),
                field("series", DataType::Utf8, false),
                field("value", DataType::Float64, true),
            ]),
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.category_index as u64),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| category_json(&row.category)),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.series.as_str()),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|row| row.value).collect::<Vec<_>>(),
                )),
            ],
        ),
        PlotData::Scatter(rows) => record_batch(
            schema(vec![
                field("ordinal", DataType::UInt64, false),
                field("series", DataType::Utf8, false),
                field("x", DataType::Float64, false),
                field("y", DataType::Float64, false),
            ]),
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.ordinal),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.series.as_str()),
                )),
                Arc::new(Float64Array::from_iter_values(rows.iter().map(|row| row.x))),
                Arc::new(Float64Array::from_iter_values(rows.iter().map(|row| row.y))),
            ],
        ),
        PlotData::Heatmap(rows) => record_batch(
            schema(vec![
                field("x_index", DataType::UInt64, false),
                field("x_category_json", DataType::Utf8, false),
                field("y_index", DataType::UInt64, false),
                field("y_category_json", DataType::Utf8, false),
                field("value", DataType::Float64, false),
            ]),
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.x_index as u64),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| category_json(&row.x_category)),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.y_index as u64),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| category_json(&row.y_category)),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.value),
                )),
            ],
        ),
        PlotData::Boxplot(rows) => record_batch(
            schema(vec![
                field("category_index", DataType::UInt64, false),
                field("category_json", DataType::Utf8, false),
                field("series", DataType::Utf8, false),
                field("min", DataType::Float64, false),
                field("q1", DataType::Float64, false),
                field("median", DataType::Float64, false),
                field("q3", DataType::Float64, false),
                field("max", DataType::Float64, false),
                field("outliers", DataType::Utf8, false),
            ]),
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.category_index as u64),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| category_json(&row.category)),
                )),
                Arc::new(StringArray::from_iter_values(
                    rows.iter().map(|row| row.series.as_str()),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.values[0]),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.values[1]),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.values[2]),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.values[3]),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.values[4]),
                )),
                Arc::new(StringArray::from_iter_values(rows.iter().map(|row| {
                    serde_json::to_string(&row.outliers).expect("finite outliers serialize")
                }))),
            ],
        ),
        PlotData::Histogram(rows) => record_batch(
            schema(vec![
                field("bin_index", DataType::UInt64, false),
                field("bin_start", DataType::Float64, false),
                field("bin_end", DataType::Float64, false),
                field("count", DataType::UInt64, false),
            ]),
            vec![
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.bin_index as u64),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.start),
                )),
                Arc::new(Float64Array::from_iter_values(
                    rows.iter().map(|row| row.end),
                )),
                Arc::new(UInt64Array::from_iter_values(
                    rows.iter().map(|row| row.count),
                )),
            ],
        ),
    }
}

fn decode_model(mut model: ChartModel, batches: &[RecordBatch]) -> Result<ChartModel, ChartError> {
    match &mut model {
        ChartModel::Line { series, .. } => {
            let mut order = Vec::new();
            let mut grouped = HashMap::<String, Vec<LinePoint>>::new();
            for batch in batches {
                let ordinal = uint64(batch, "ordinal")?;
                let x_kind = string(batch, "x_kind")?;
                let x_category = string(batch, "x_category_json")?;
                let x_value = float64(batch, "x_value")?;
                let names = string(batch, "series")?;
                let values = float64(batch, "value")?;
                for row in 0..batch.num_rows() {
                    let name = names.value(row).to_owned();
                    if !grouped.contains_key(&name) {
                        order.push(name.clone());
                    }
                    let x = match x_kind.value(row) {
                        "category" => LineXValue::Category(parse_category(x_category.value(row))?),
                        "value" => LineXValue::Number(x_value.value(row)),
                        _ => return Err(materialization("未知 line x_kind")),
                    };
                    grouped.entry(name).or_default().push(LinePoint {
                        x,
                        y: values.value(row),
                        ordinal: ordinal.value(row),
                    });
                }
            }
            *series = order
                .into_iter()
                .map(|name| LineSeries {
                    points: grouped.remove(&name).unwrap_or_default(),
                    name,
                })
                .collect();
        }
        ChartModel::Bar {
            categories, series, ..
        } => {
            let mut rows = Vec::new();
            let mut category_map = HashMap::new();
            let mut order = Vec::new();
            for batch in batches {
                let indices = uint64(batch, "category_index")?;
                let category_values = string(batch, "category_json")?;
                let names = string(batch, "series")?;
                let values = float64(batch, "value")?;
                for row in 0..batch.num_rows() {
                    let index = indices.value(row) as usize;
                    category_map
                        .entry(index)
                        .or_insert(parse_category(category_values.value(row))?);
                    let name = names.value(row).to_owned();
                    if !order.contains(&name) {
                        order.push(name.clone());
                    }
                    rows.push((
                        index,
                        name,
                        (!values.is_null(row)).then(|| values.value(row)),
                    ));
                }
            }
            let count = category_map.keys().max().map_or(0, |index| index + 1);
            *categories = (0..count)
                .map(|index| {
                    category_map
                        .remove(&index)
                        .ok_or_else(|| materialization("bar category index 不连续"))
                })
                .collect::<Result<_, _>>()?;
            *series = order
                .into_iter()
                .map(|name| {
                    let mut values = vec![None; count];
                    for (index, row_name, value) in &rows {
                        if *row_name == name {
                            values[*index] = *value;
                        }
                    }
                    BarSeries { name, values }
                })
                .collect();
        }
        ChartModel::Scatter { series, .. } => {
            let mut order = Vec::new();
            let mut grouped = HashMap::<String, Vec<super::NumericPoint>>::new();
            for batch in batches {
                let ordinals = uint64(batch, "ordinal")?;
                let names = string(batch, "series")?;
                let x = float64(batch, "x")?;
                let y = float64(batch, "y")?;
                for row in 0..batch.num_rows() {
                    let name = names.value(row).to_owned();
                    if !grouped.contains_key(&name) {
                        order.push(name.clone());
                    }
                    grouped.entry(name).or_default().push(super::NumericPoint {
                        x: x.value(row),
                        y: y.value(row),
                        ordinal: ordinals.value(row),
                    });
                }
            }
            *series = order
                .into_iter()
                .map(|name| ScatterSeries {
                    points: grouped.remove(&name).unwrap_or_default(),
                    name,
                })
                .collect();
        }
        ChartModel::Heatmap {
            x_axis,
            y_axis,
            x_categories,
            y_categories,
            cells,
            ..
        } => {
            let mut x_map = HashMap::new();
            let mut y_map = HashMap::new();
            cells.clear();
            for batch in batches {
                let x_indices = uint64(batch, "x_index")?;
                let x_values = string(batch, "x_category_json")?;
                let y_indices = uint64(batch, "y_index")?;
                let y_values = string(batch, "y_category_json")?;
                let values = float64(batch, "value")?;
                for row in 0..batch.num_rows() {
                    let x_index = x_indices.value(row) as usize;
                    let y_index = y_indices.value(row) as usize;
                    x_map
                        .entry(x_index)
                        .or_insert(parse_category(x_values.value(row))?);
                    y_map
                        .entry(y_index)
                        .or_insert(parse_category(y_values.value(row))?);
                    cells.push(HeatmapCell {
                        x_index,
                        y_index,
                        value: values.value(row),
                    });
                }
            }
            *x_categories = ordered_categories(x_map)?;
            *y_categories = ordered_categories(y_map)?;
            x_axis.categories.clone_from(x_categories);
            y_axis.categories.clone_from(y_categories);
        }
        ChartModel::Boxplot {
            category_axis,
            series,
            ..
        } => {
            let mut category_map = HashMap::new();
            let mut rows = Vec::new();
            let mut order = Vec::new();
            for batch in batches {
                let indices = uint64(batch, "category_index")?;
                let category_values = string(batch, "category_json")?;
                let names = string(batch, "series")?;
                let min = float64(batch, "min")?;
                let q1 = float64(batch, "q1")?;
                let median = float64(batch, "median")?;
                let q3 = float64(batch, "q3")?;
                let max = float64(batch, "max")?;
                let outliers = string(batch, "outliers")?;
                for row in 0..batch.num_rows() {
                    let index = indices.value(row) as usize;
                    category_map
                        .entry(index)
                        .or_insert(parse_category(category_values.value(row))?);
                    let name = names.value(row).to_owned();
                    if !order.contains(&name) {
                        order.push(name.clone());
                    }
                    rows.push((
                        index,
                        name,
                        [
                            min.value(row),
                            q1.value(row),
                            median.value(row),
                            q3.value(row),
                            max.value(row),
                        ],
                        serde_json::from_str::<Vec<f64>>(outliers.value(row))
                            .map_err(|error| materialization(error.to_string()))?,
                    ));
                }
            }
            category_axis.categories = ordered_categories(category_map)?;
            let category_count = category_axis.categories.len();
            *series = order
                .into_iter()
                .map(|name| {
                    let mut values = vec![None; category_count];
                    let mut outliers = Vec::new();
                    for (index, row_name, summary, row_outliers) in &rows {
                        if *row_name == name {
                            values[*index] = Some(*summary);
                            outliers.extend(row_outliers.iter().map(|value| BoxplotOutlier {
                                category_index: *index,
                                value: *value,
                            }));
                        }
                    }
                    BoxplotSeries {
                        name,
                        values,
                        outliers,
                    }
                })
                .collect();
        }
        ChartModel::Histogram { bins, .. } => {
            let mut indexed = Vec::new();
            for batch in batches {
                let indices = uint64(batch, "bin_index")?;
                let starts = float64(batch, "bin_start")?;
                let ends = float64(batch, "bin_end")?;
                let counts = uint64(batch, "count")?;
                for row in 0..batch.num_rows() {
                    indexed.push((
                        indices.value(row) as usize,
                        starts.value(row),
                        ends.value(row),
                        counts.value(row),
                    ));
                }
            }
            indexed.sort_by_key(|row| row.0);
            let last = indexed.len().saturating_sub(1);
            *bins = indexed
                .into_iter()
                .enumerate()
                .map(|(position, (_, start, end, count))| HistogramBin {
                    start,
                    end,
                    count,
                    includes_end: position == last,
                })
                .collect();
        }
    }
    Ok(model)
}

fn expected_columns(model: &ChartModel) -> &'static [&'static str] {
    match model {
        ChartModel::Line { .. } => &[
            "ordinal",
            "x_kind",
            "x_category_json",
            "x_value",
            "series",
            "value",
        ],
        ChartModel::Bar { .. } => &["category_index", "category_json", "series", "value"],
        ChartModel::Scatter { .. } => &["ordinal", "series", "x", "y"],
        ChartModel::Heatmap { .. } => &[
            "x_index",
            "x_category_json",
            "y_index",
            "y_category_json",
            "value",
        ],
        ChartModel::Boxplot { .. } => &[
            "category_index",
            "category_json",
            "series",
            "min",
            "q1",
            "median",
            "q3",
            "max",
            "outliers",
        ],
        ChartModel::Histogram { .. } => &["bin_index", "bin_start", "bin_end", "count"],
    }
}

fn validate_columns(schema: &Schema, expected: &[&str]) -> Result<(), ChartError> {
    let actual = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect::<Vec<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(materialization("Plot Data schema 与图表类型不匹配"))
    }
}

fn ordered_categories(
    mut values: HashMap<usize, CategoryValue>,
) -> Result<Vec<CategoryValue>, ChartError> {
    let count = values.keys().max().map_or(0, |index| index + 1);
    (0..count)
        .map(|index| {
            values
                .remove(&index)
                .ok_or_else(|| materialization("category index 不连续"))
        })
        .collect()
}

fn schema(fields: Vec<Field>) -> Arc<Schema> {
    Arc::new(Schema::new(fields))
}

fn field(name: &str, data_type: DataType, nullable: bool) -> Field {
    Field::new(name, data_type, nullable)
}

fn record_batch(
    schema: Arc<Schema>,
    columns: Vec<Arc<dyn Array>>,
) -> Result<RecordBatch, ChartError> {
    RecordBatch::try_new(schema, columns).map_err(|error| materialization(error.to_string()))
}

fn uint64<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a UInt64Array, ChartError> {
    typed_column(batch, name)
}

fn float64<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a Float64Array, ChartError> {
    typed_column(batch, name)
}

fn string<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a StringArray, ChartError> {
    typed_column(batch, name)
}

fn typed_column<'a, T: 'static>(batch: &'a RecordBatch, name: &str) -> Result<&'a T, ChartError> {
    let index = batch
        .schema()
        .index_of(name)
        .map_err(|error| materialization(error.to_string()))?;
    batch
        .column(index)
        .as_any()
        .downcast_ref::<T>()
        .ok_or_else(|| materialization(format!("Plot Data 列类型错误: {name}")))
}

fn category_json(value: &CategoryValue) -> String {
    serde_json::to_string(value).expect("finite CategoryValue serializes")
}

fn parse_category(value: &str) -> Result<CategoryValue, ChartError> {
    serde_json::from_str(value).map_err(|error| materialization(error.to_string()))
}

fn plot_row_count(plot_data: &PlotData) -> usize {
    match plot_data {
        PlotData::Line(rows) => rows.len(),
        PlotData::Bar(rows) => rows.len(),
        PlotData::Scatter(rows) => rows.len(),
        PlotData::Heatmap(rows) => rows.len(),
        PlotData::Boxplot(rows) => rows.len(),
        PlotData::Histogram(rows) => rows.len(),
    }
}

fn materialization(message: impl Into<String>) -> ChartError {
    ChartError::new("chart_materialization_failed", message)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use parquet::basic::Compression;
    use serde_json::{Value, json};

    use super::*;
    use crate::application::charting::{ChartCompiler, ChartConfig, ChartSource};
    use crate::application::dataframe_reader::DataFrameColumn;
    use crate::storage::ArtifactStore;

    fn presentation() -> Value {
        json!({
            "legend":{"show":true,"position":"auto","orientation":"auto"},
            "palette":"colorblind","title":{"show":true},"axis":{"showGrid":true}
        })
    }

    fn line_presentation() -> Value {
        let mut value = presentation();
        value["line"] = json!({"width":2,"style":"solid","showSymbol":false});
        value
    }

    fn config(value: Value) -> ChartConfig {
        serde_json::from_value(value).unwrap()
    }

    fn column(name: &str, dtype: &str) -> DataFrameColumn {
        DataFrameColumn {
            name: name.into(),
            dtype: dtype.into(),
            nullable: true,
            unit: None,
        }
    }

    fn source(columns: Vec<DataFrameColumn>, arrays: Vec<(&str, ArrayRef)>) -> ChartSource {
        ChartSource::new(
            "dataset-7",
            columns,
            vec![RecordBatch::try_from_iter(arrays).unwrap()],
        )
    }

    fn fixtures() -> Vec<crate::application::charting::CompiledChart> {
        let categorical = source(
            vec![column("bus", "Utf8"), column("value", "Float64")],
            vec![
                ("bus", Arc::new(StringArray::from(vec!["b1", "b2"]))),
                ("value", Arc::new(Float64Array::from(vec![1.0, 2.0]))),
            ],
        );
        let numeric = source(
            vec![column("x", "Float64"), column("value", "Float64")],
            vec![
                ("x", Arc::new(Float64Array::from(vec![1.0, 2.0]))),
                ("value", Arc::new(Float64Array::from(vec![3.0, 4.0]))),
            ],
        );
        let heatmap = source(
            vec![
                column("x", "Utf8"),
                column("y", "Utf8"),
                column("value", "Float64"),
            ],
            vec![
                ("x", Arc::new(StringArray::from(vec!["a", "b"]))),
                ("y", Arc::new(StringArray::from(vec!["r", "s"]))),
                ("value", Arc::new(Float64Array::from(vec![5.0, 6.0]))),
            ],
        );

        vec![
            ChartCompiler::compile(
                &config(json!({
                    "version":1,"kind":"line","presentation":line_presentation(),
                    "x":{"column":"bus"},"xAxisMode":"category","measures":[{"column":"value"}],
                    "order":"source","aggregation":"none","sampling":{"mode":"none"}
                })),
                &categorical,
            )
            .unwrap(),
            ChartCompiler::compile(
                &config(json!({
                    "version":1,"kind":"bar","presentation":presentation(),
                    "x":{"column":"bus"},"measures":[{"column":"value"}],
                    "order":"source","aggregation":"none"
                })),
                &categorical,
            )
            .unwrap(),
            ChartCompiler::compile(
                &config(json!({
                    "version":1,"kind":"scatter","presentation":presentation(),
                    "x":{"column":"x"},"measures":[{"column":"value"}]
                })),
                &numeric,
            )
            .unwrap(),
            ChartCompiler::compile(
                &config(json!({
                    "version":1,"kind":"heatmap","presentation":presentation(),
                    "x":{"column":"x"},"y":{"column":"y"},"value":{"column":"value"},
                    "aggregation":"mean"
                })),
                &heatmap,
            )
            .unwrap(),
            ChartCompiler::compile(
                &config(json!({
                    "version":1,"kind":"boxplot","presentation":presentation(),
                    "category":{"column":"bus"},"value":{"column":"value"},"showOutliers":true
                })),
                &categorical,
            )
            .unwrap(),
            ChartCompiler::compile(
                &config(json!({
                    "version":1,"kind":"histogram","presentation":presentation(),
                    "value":{"column":"value"},"bins":2
                })),
                &numeric,
            )
            .unwrap(),
        ]
    }

    #[test]
    fn every_kind_round_trips_through_narrow_snappy_plot_data() {
        for compiled in fixtures() {
            let bytes = PlotDataCodec::encode_snappy(&compiled).unwrap();
            let mut temporary = tempfile::NamedTempFile::new().unwrap();
            temporary.write_all(&bytes).unwrap();
            temporary.flush().unwrap();
            let builder =
                ParquetRecordBatchReaderBuilder::try_new(File::open(temporary.path()).unwrap())
                    .unwrap();
            assert!(
                builder
                    .metadata()
                    .row_groups()
                    .iter()
                    .flat_map(|group| group.columns())
                    .all(|column| column.compression() == Compression::SNAPPY)
            );
            let actual_names = builder
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>();
            let expected_names = compiled
                .plot_schema
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual_names, expected_names);
            let decoded =
                PlotDataCodec::read_model(File::open(temporary.path()).unwrap(), &compiled.model)
                    .unwrap();
            assert_eq!(decoded, compiled.model);
        }
    }

    #[test]
    fn materialization_stages_hashes_and_counts_the_plot_artifact() {
        let root = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(root.path().to_path_buf()).unwrap();
        let compiled = fixtures().remove(0);
        let materialized = PlotDataCodec::write_snappy(&store, &compiled).unwrap();

        assert_eq!(materialized.content_hash.len(), 64);
        assert_eq!(materialized.content_hash, materialized.staged.sha256);
        assert_eq!(materialized.size_bytes, materialized.staged.size_bytes);
        assert_eq!(materialized.point_count, compiled.diagnostics.output_points);
        assert!(materialized.row_count > 0);
        assert_eq!(materialized.schema_hash.len(), 64);
        assert_eq!(
            serde_json::from_str::<Value>(&materialized.schema_json).unwrap(),
            serde_json::to_value(&compiled.plot_schema).unwrap()
        );
    }
}
