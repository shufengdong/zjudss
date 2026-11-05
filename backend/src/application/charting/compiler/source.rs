use super::*;

#[derive(Debug, Clone)]
pub struct ChartSource {
    pub dataset_id: String,
    pub columns: Vec<DataFrameColumn>,
    pub batches: Vec<RecordBatch>,
}

impl ChartSource {
    pub fn new(
        dataset_id: impl Into<String>,
        columns: Vec<DataFrameColumn>,
        batches: Vec<RecordBatch>,
    ) -> Self {
        Self {
            dataset_id: dataset_id.into(),
            columns,
            batches,
        }
    }
}
