//! 资源树的安全遍历、有界读取和字节级摘要工具。
//!
//! 所有路径都必须留在配置根目录内；符号链接、特殊文件和超限内容会被拒绝。

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::EngineResourceError;
use super::snapshot::SourceFileSpec;

pub(crate) fn portable_relative(root: &Path, path: &Path) -> Result<String, EngineResourceError> {
    path.strip_prefix(root)
        .ok()
        .and_then(Path::to_str)
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| EngineResourceError::new("engine_resource_path_escape", "路径不在资源根内"))
}

pub(crate) fn discover_documents(
    dss_root: &Path,
    max_documents: usize,
) -> Result<Vec<PathBuf>, EngineResourceError> {
    let docs_root = safe_directory(dss_root, "docs")?;
    let mut files = Vec::new();
    walk_matching_files(&docs_root, true, max_documents, is_markdown, &mut files)?;
    files.sort_by_key(|path| portable_relative(dss_root, path).unwrap_or_default());
    Ok(files)
}

pub(crate) fn discover_bundle_manifests(
    dss_root: &Path,
    max_bundles: usize,
    max_example_bytes: u64,
) -> Result<Vec<SourceFileSpec>, EngineResourceError> {
    let root = dss_root.join("rssbundle");
    if !root.exists() {
        return Ok(Vec::new());
    }
    if std::fs::symlink_metadata(&root)
        .map_err(EngineResourceError::io)?
        .file_type()
        .is_symlink()
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            "rssbundle",
        ));
    }
    let mut files = Vec::new();
    walk_bundle_manifests(&root, max_bundles, &mut files)?;
    files.sort_by_key(|path| portable_relative(dss_root, path).unwrap_or_default());
    files
        .into_iter()
        .map(|path| {
            Ok(SourceFileSpec {
                path: portable_relative(dss_root, &path)?,
                sha256: hash_bounded_file(&path, max_example_bytes, "example bytes")?,
            })
        })
        .collect()
}

fn walk_bundle_manifests(
    root: &Path,
    max_bundles: usize,
    manifests: &mut Vec<PathBuf>,
) -> Result<(), EngineResourceError> {
    let mut entries = std::fs::read_dir(root)
        .map_err(EngineResourceError::io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(EngineResourceError::io)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(EngineResourceError::io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rssbundle"))
        {
            let manifest = path.join("manifest.json");
            if manifest.is_file() {
                if manifests.len() >= max_bundles {
                    return Err(limit_error("example count"));
                }
                manifests.push(manifest);
            }
            continue;
        }
        walk_bundle_manifests(&path, max_bundles, manifests)?;
    }
    Ok(())
}

fn walk_matching_files(
    root: &Path,
    reject_symlinks: bool,
    max_files: usize,
    matches: fn(&Path) -> bool,
    files: &mut Vec<PathBuf>,
) -> Result<(), EngineResourceError> {
    for entry in std::fs::read_dir(root).map_err(EngineResourceError::io)? {
        let entry = entry.map_err(EngineResourceError::io)?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(EngineResourceError::io)?;
        if metadata.file_type().is_symlink() {
            if reject_symlinks {
                return Err(EngineResourceError::new(
                    "engine_resource_path_escape",
                    "资源树包含符号链接",
                ));
            }
            continue;
        }
        if metadata.is_dir() {
            walk_matching_files(&path, reject_symlinks, max_files, matches, files)?;
        } else if metadata.is_file() && matches(&path) {
            if files.len() >= max_files {
                return Err(limit_error("resource file count"));
            }
            files.push(path);
        }
    }
    Ok(())
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

pub(crate) fn read_safe_bounded(
    root: &Path,
    relative: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, EngineResourceError> {
    read_bounded_file(&safe_file(root, relative)?, max_bytes, "catalog file size")
}

pub(crate) fn read_bounded_file(
    path: &Path,
    max_bytes: u64,
    kind: &str,
) -> Result<Vec<u8>, EngineResourceError> {
    if std::fs::metadata(path)
        .map_err(EngineResourceError::io)?
        .len()
        > max_bytes
    {
        return Err(limit_error(kind));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(EngineResourceError::io)?
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(EngineResourceError::io)?;
    if bytes.len() as u64 > max_bytes {
        return Err(limit_error(kind));
    }
    Ok(bytes)
}

pub(crate) fn hash_file(path: &Path) -> Result<String, EngineResourceError> {
    let mut file = std::fs::File::open(path).map_err(EngineResourceError::io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(EngineResourceError::io)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn hash_bounded_file(
    path: &Path,
    max_bytes: u64,
    kind: &str,
) -> Result<String, EngineResourceError> {
    hash_bounded_file_with_size(path, max_bytes, kind).map(|(hash, _)| hash)
}

pub(crate) fn hash_bounded_file_with_size(
    path: &Path,
    max_bytes: u64,
    kind: &str,
) -> Result<(String, u64), EngineResourceError> {
    let expected_size = std::fs::metadata(path)
        .map_err(EngineResourceError::io)?
        .len();
    if expected_size > max_bytes {
        return Err(limit_error(kind));
    }
    let mut file = std::fs::File::open(path).map_err(EngineResourceError::io)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(EngineResourceError::io)?;
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| limit_error(kind))?;
        if size > max_bytes {
            return Err(limit_error(kind));
        }
        hasher.update(&buffer[..count]);
    }
    if size != expected_size {
        return Err(EngineResourceError::new(
            "engine_resource_changed_during_scan",
            "resource changed while hashing",
        ));
    }
    Ok((hex::encode(hasher.finalize()), size))
}

pub(crate) fn safe_file(root: &Path, relative: &str) -> Result<PathBuf, EngineResourceError> {
    safe_path(root, relative, false)
}

pub(crate) fn safe_directory(root: &Path, relative: &str) -> Result<PathBuf, EngineResourceError> {
    safe_path(root, relative, true)
}

fn safe_path(root: &Path, relative: &str, directory: bool) -> Result<PathBuf, EngineResourceError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            relative,
        ));
    }
    let mut current = root.to_path_buf();
    for component in relative_path.components() {
        if let Component::Normal(part) = component {
            current.push(part);
            let metadata = std::fs::symlink_metadata(&current).map_err(EngineResourceError::io)?;
            if metadata.file_type().is_symlink() {
                return Err(EngineResourceError::new(
                    "engine_resource_path_escape",
                    relative,
                ));
            }
        }
    }
    let canonical = current.canonicalize().map_err(EngineResourceError::io)?;
    if !canonical.starts_with(root)
        || (directory && !canonical.is_dir())
        || (!directory && !canonical.is_file())
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            relative,
        ));
    }
    Ok(canonical)
}

pub(crate) fn limit_error(kind: &str) -> EngineResourceError {
    EngineResourceError::new(
        "engine_resource_limit_exceeded",
        format!("engine resource exceeds {kind} limit"),
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use super::{
        discover_bundle_manifests, hash_bounded_file_with_size, hash_bytes, read_bounded_file,
        safe_file,
    };

    #[test]
    fn bounded_read_rejects_sparse_file_from_metadata() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("large.md");
        let mut file = std::fs::File::create(&path).unwrap();
        file.seek(SeekFrom::Start(1024 * 1024)).unwrap();
        file.write_all(&[0]).unwrap();
        let error = read_bounded_file(&path, 1024, "test file").unwrap_err();
        assert_eq!(error.code(), "engine_resource_limit_exceeded");
    }

    #[test]
    fn bounded_hash_reports_the_streamed_size_and_digest() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.dss");
        let content = vec![b'x'; 3 * 1024 * 1024];
        std::fs::write(&path, &content).unwrap();

        let (digest, size) =
            hash_bounded_file_with_size(&path, content.len() as u64, "test file").unwrap();

        assert_eq!(digest, hash_bytes(&content));
        assert_eq!(size, content.len() as u64);
        assert_eq!(
            hash_bounded_file_with_size(&path, content.len() as u64 - 1, "test file")
                .unwrap_err()
                .code(),
            "engine_resource_limit_exceeded"
        );
    }

    #[cfg(unix)]
    #[test]
    fn safe_tree_paths_reject_symlink_components() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("model.dss"), "outside").unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();

        let error = safe_file(root.path(), "linked/model.dss").unwrap_err();

        assert_eq!(error.code(), "engine_resource_path_escape");
    }

    #[test]
    fn bundle_discovery_is_sorted_and_stops_at_bundle_leaf() {
        let root = tempfile::tempdir().unwrap();
        let dss = root.path();
        for path in [
            "rssbundle/B/z.rssbundle/manifest.json",
            "rssbundle/A/a.rssbundle/manifest.json",
            "rssbundle/B/z.rssbundle/nested/ignored.rssbundle/manifest.json",
        ] {
            let path = dss.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "{}").unwrap();
        }

        let manifests = discover_bundle_manifests(dss, 10, 1024).unwrap();

        assert_eq!(
            manifests
                .into_iter()
                .map(|manifest| manifest.path)
                .collect::<Vec<_>>(),
            [
                "rssbundle/A/a.rssbundle/manifest.json",
                "rssbundle/B/z.rssbundle/manifest.json"
            ]
        );
    }
}
