use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::IngestionFormatError;

#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_entries: usize,
    pub max_expanded_bytes: u64,
    pub max_compression_ratio: u64,
    pub max_file_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 1_000,
            max_expanded_bytes: 2 * 1024 * 1024 * 1024,
            max_compression_ratio: 100,
            max_file_bytes: 50 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TreeFile {
    pub relative_path: String,
    pub path: PathBuf,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct ExtractedTree {
    pub root: PathBuf,
    pub files: BTreeMap<String, TreeFile>,
}

impl ExtractedTree {
    pub fn from_files(
        root: PathBuf,
        paths: impl IntoIterator<Item = (String, PathBuf)>,
    ) -> Result<Self, IngestionFormatError> {
        let mut files = BTreeMap::new();
        for (relative, path) in paths {
            let relative = safe_relative(&relative)?;
            if files.contains_key(&relative) {
                return Err(IngestionFormatError::new(
                    "archive_duplicate_path",
                    relative,
                ));
            }
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| IngestionFormatError::io("tree_read_failed", error))?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(IngestionFormatError::new(
                    "archive_symlink_forbidden",
                    relative,
                ));
            }
            let bytes = std::fs::read(&path)
                .map_err(|error| IngestionFormatError::io("tree_read_failed", error))?;
            files.insert(
                relative.clone(),
                TreeFile {
                    relative_path: relative.clone(),
                    path,
                    size_bytes: metadata.len(),
                    sha256: format!("{:x}", Sha256::digest(&bytes)),
                },
            );
        }
        Ok(Self { root, files })
    }
}

pub struct ArchiveExtractor;

impl ArchiveExtractor {
    pub fn extract(
        zip_path: &Path,
        destination: &Path,
        limits: ArchiveLimits,
    ) -> Result<ExtractedTree, IngestionFormatError> {
        if destination.exists() {
            std::fs::remove_dir_all(destination)
                .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
        }
        std::fs::create_dir_all(destination)
            .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
        let result = extract_inner(zip_path, destination, limits);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(destination);
        }
        result
    }
}

fn extract_inner(
    zip_path: &Path,
    destination: &Path,
    limits: ArchiveLimits,
) -> Result<ExtractedTree, IngestionFormatError> {
    validate_central_names(zip_path)?;
    let file = File::open(zip_path)
        .map_err(|error| IngestionFormatError::io("archive_read_failed", error))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| IngestionFormatError::new("archive_invalid", error.to_string()))?;
    if archive.len() > limits.max_entries {
        return Err(IngestionFormatError::new(
            "archive_too_many_entries",
            archive.len().to_string(),
        ));
    }
    let mut seen = HashSet::new();
    let mut total = 0_u64;
    let mut files = BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| IngestionFormatError::new("archive_invalid", error.to_string()))?;
        let raw_name = std::str::from_utf8(entry.name_raw())
            .map_err(|error| IngestionFormatError::new("archive_path_invalid", error.to_string()))?
            .to_string();
        if raw_name.contains('\0') || raw_name.contains('\\') {
            return Err(IngestionFormatError::new("archive_path_invalid", raw_name));
        }
        // `enclosed_name` may return a safe transformed view for some ZIP producers;
        // validate the unmodified entry name as well so absolute and dot components are rejected.
        safe_relative(raw_name.trim_end_matches('/'))?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(IngestionFormatError::new(
                "archive_symlink_forbidden",
                raw_name,
            ));
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| IngestionFormatError::new("archive_path_escape", raw_name.clone()))?;
        let relative = safe_relative(&enclosed.to_string_lossy())?;
        if !seen.insert(relative.clone()) {
            return Err(IngestionFormatError::new(
                "archive_duplicate_path",
                relative,
            ));
        }
        if entry.is_dir() {
            continue;
        }
        if entry.size() > limits.max_file_bytes {
            return Err(IngestionFormatError::new(
                "archive_file_too_large",
                relative,
            ));
        }
        total = total.checked_add(entry.size()).ok_or_else(|| {
            IngestionFormatError::new("archive_expanded_too_large", "size overflow")
        })?;
        if total > limits.max_expanded_bytes {
            return Err(IngestionFormatError::new(
                "archive_expanded_too_large",
                total.to_string(),
            ));
        }
        let compressed = entry.compressed_size().max(1);
        if entry.size() / compressed > limits.max_compression_ratio {
            return Err(IngestionFormatError::new(
                "archive_compression_ratio_exceeded",
                relative,
            ));
        }
        let output = destination.join(&relative);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
        }
        let mut target = File::create(&output)
            .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
        let mut hasher = Sha256::new();
        let mut written = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = entry
                .read(&mut buffer)
                .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > limits.max_file_bytes
                || total - entry.size() + written > limits.max_expanded_bytes
            {
                return Err(IngestionFormatError::new(
                    "archive_expanded_too_large",
                    relative,
                ));
            }
            target
                .write_all(&buffer[..count])
                .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
            hasher.update(&buffer[..count]);
        }
        target
            .sync_all()
            .map_err(|error| IngestionFormatError::io("archive_extract_failed", error))?;
        if files
            .insert(
                relative.clone(),
                TreeFile {
                    relative_path: relative.clone(),
                    path: output,
                    size_bytes: written,
                    sha256: format!("{:x}", hasher.finalize()),
                },
            )
            .is_some()
        {
            return Err(IngestionFormatError::new(
                "archive_duplicate_path",
                relative,
            ));
        }
    }
    Ok(ExtractedTree {
        root: destination.to_path_buf(),
        files,
    })
}

fn validate_central_names(zip_path: &Path) -> Result<(), IngestionFormatError> {
    let bytes = std::fs::read(zip_path)
        .map_err(|error| IngestionFormatError::io("archive_read_failed", error))?;
    let search_start = bytes.len().saturating_sub(65_557);
    let eocd = (search_start..bytes.len().saturating_sub(3))
        .rev()
        .find(|&index| bytes[index..index + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or_else(|| IngestionFormatError::new("archive_invalid", "missing end record"))?;
    if eocd + 20 > bytes.len() {
        return Err(IngestionFormatError::new(
            "archive_invalid",
            "truncated end record",
        ));
    }
    let read_u32 =
        |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
    let central_size = read_u32(eocd + 12);
    let mut cursor = read_u32(eocd + 16);
    let end = cursor
        .checked_add(central_size)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| {
            IngestionFormatError::new("archive_invalid", "central directory outside file")
        })?;
    let mut seen = HashSet::new();
    while cursor < end {
        if cursor + 46 > end || bytes[cursor..cursor + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(IngestionFormatError::new(
                "archive_invalid",
                "invalid central entry",
            ));
        }
        let u16_at = |offset: usize| {
            u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap()) as usize
        };
        let name_len = u16_at(cursor + 28);
        let extra_len = u16_at(cursor + 30);
        let comment_len = u16_at(cursor + 32);
        let name_start = cursor + 46;
        let next = name_start
            .checked_add(name_len + extra_len + comment_len)
            .filter(|next| *next <= end)
            .ok_or_else(|| {
                IngestionFormatError::new("archive_invalid", "truncated central entry")
            })?;
        let raw =
            std::str::from_utf8(&bytes[name_start..name_start + name_len]).map_err(|error| {
                IngestionFormatError::new("archive_path_invalid", error.to_string())
            })?;
        let normalized = safe_relative(raw.trim_end_matches('/'))?;
        if !seen.insert(normalized.clone()) {
            return Err(IngestionFormatError::new(
                "archive_duplicate_path",
                normalized,
            ));
        }
        cursor = next;
    }
    Ok(())
}

pub fn safe_relative(value: &str) -> Result<String, IngestionFormatError> {
    if value.is_empty() || value.contains('\0') || value.contains('\\') {
        return Err(IngestionFormatError::new("archive_path_invalid", value));
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(IngestionFormatError::new("archive_path_escape", value));
    }
    Ok(path
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}
