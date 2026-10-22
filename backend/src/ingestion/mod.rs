mod tabular;

pub use tabular::{
    ColumnSchema, NormalizedSheet, TableSchema, TabularError, TabularFormat, TabularInspector,
    normalize_excel_to_parquet,
};
