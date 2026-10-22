use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_STAGING_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn unique_staging_path(directory: &Path) -> PathBuf {
    let sequence = NEXT_STAGING_ID.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        "stage-{}-{}-{sequence}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}
