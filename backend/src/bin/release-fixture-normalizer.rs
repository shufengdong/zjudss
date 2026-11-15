use std::fs::File;
use std::sync::Arc;

use arrow_array::builder::{LargeListBuilder, StringViewBuilder};
use arrow_array::{Array, ArrayRef, LargeListArray, RecordBatch, StringViewArray};
use arrow_schema::DataType;
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let input = arguments.next().ok_or("missing input parquet")?;
    let output = arguments.next().ok_or("missing output parquet")?;
    if arguments.next().is_some() {
        return Err("unexpected argument".into());
    }

    let reader = ParquetRecordBatchReaderBuilder::try_new(File::open(input)?)?;
    let schema = reader.schema().clone();
    let terminal_index = schema.index_of("terminals")?;
    let element_field = match schema.field(terminal_index).data_type() {
        DataType::LargeList(field) if field.data_type() == &DataType::Utf8View => field.clone(),
        other => return Err(format!("unexpected terminals type: {other}").into()),
    };
    let batches = reader.build()?.collect::<Result<Vec<_>, _>>()?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(File::create(output)?, schema.clone(), Some(properties))?;
    for batch in batches {
        let terminals = batch
            .column(terminal_index)
            .as_any()
            .downcast_ref::<LargeListArray>()
            .ok_or("terminals is not a LargeList array")?;
        let mut builder =
            LargeListBuilder::new(StringViewBuilder::new()).with_field(element_field.clone());
        for row in 0..terminals.len() {
            if terminals.is_null(row) {
                builder.append(false);
                continue;
            }
            let values = terminals.value(row);
            let values = values
                .as_any()
                .downcast_ref::<StringViewArray>()
                .ok_or("terminals values are not Utf8View")?;
            for index in 0..values.len().min(3) {
                if values.is_null(index) {
                    builder.values().append_null();
                } else {
                    builder.values().append_value(values.value(index));
                }
            }
            builder.append(true);
        }
        let mut columns = batch.columns().to_vec();
        columns[terminal_index] = Arc::new(builder.finish()) as ArrayRef;
        writer.write(&RecordBatch::try_new(schema.clone(), columns)?)?;
    }
    writer.close()?;
    Ok(())
}
