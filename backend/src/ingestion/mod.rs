mod archive;
mod bundle;
mod rss;
mod tabular;

pub const UPLOAD_SOURCES: &[&str] = &[
    "single_rss",
    "zip_rssbundle",
    "directory_rssbundle",
    "csv",
    "parquet",
    "excel",
];

#[derive(Debug)]
pub struct IngestionFormatError {
    code: &'static str,
    message: String,
}

impl IngestionFormatError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub(crate) fn io(code: &'static str, error: std::io::Error) -> Self {
        Self::new(code, error.to_string())
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}
impl std::fmt::Display for IngestionFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for IngestionFormatError {}

pub use archive::{ArchiveExtractor, ArchiveLimits, ExtractedTree, TreeFile, safe_relative};
pub use bundle::{BundleFile, BundleManifest, BundleStudy, BundleValidator, ValidatedBundle};
pub use rss::{DependencyReport, RssInspector};

pub use tabular::{
    ColumnSchema, NormalizedSheet, TableSchema, TabularError, TabularFormat, TabularInspector,
    normalize_csv_to_parquet, normalize_excel_to_parquet,
};
