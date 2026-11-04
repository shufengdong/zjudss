use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::tree::{hash_bytes, hash_file, safe_directory, safe_file};
use super::{EngineResourceError, EngineResourceSnapshot};

pub(super) fn publish_snapshot(
    mut snapshot: EngineResourceSnapshot,
    data_dir: &Path,
) -> Result<(EngineResourceSnapshot, PathBuf), EngineResourceError> {
    let parent = data_dir.join("system/engine-resources");
    std::fs::create_dir_all(&parent).map_err(EngineResourceError::io)?;
    let parent = parent.canonicalize().map_err(EngineResourceError::io)?;
    let destination = parent.join(&snapshot.id);
    let generated = generated_snapshot_files(&snapshot)?;
    let mut inventory = generated
        .iter()
        .map(|(path, bytes)| (path.clone(), hash_bytes(bytes)))
        .collect::<BTreeMap<_, _>>();
    inventory.extend(
        snapshot
            .source_hashes
            .iter()
            .map(|(path, hash)| (format!("source/{path}"), hash.clone())),
    );
    if destination.is_dir() {
        verify_published_snapshot(&destination, &snapshot.id, &inventory)?;
        rebase_snapshot(&mut snapshot, &destination)?;
        return Ok((snapshot, destination));
    }
    let staging = parent.join(format!(".{}-{}", snapshot.id, std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(EngineResourceError::io)?;
    }
    let result = (|| {
        std::fs::create_dir_all(&staging).map_err(EngineResourceError::io)?;
        for (relative, bytes) in &generated {
            write_published_file(&staging, relative, bytes)?;
        }
        for (relative, expected_hash) in &snapshot.source_hashes {
            let source = safe_file(&snapshot.engine_root, relative).map_err(|_| {
                EngineResourceError::new(
                    "engine_resource_changed_during_scan",
                    format!("resource disappeared: {relative}"),
                )
            })?;
            copy_verified_published_file(
                &staging,
                &format!("source/{relative}"),
                &source,
                expected_hash,
            )?;
        }
        write_published_file(
            &staging,
            "manifest.json",
            &serde_json::to_vec_pretty(&serde_json::json!({
                "snapshotId": snapshot.id,
                "engineContractVersion": snapshot.contract_version,
                "catalogSchemaVersions": snapshot.catalog_schema_versions,
                "facades": {
                    "catalogEntries": snapshot.facades.catalog_entry_count(),
                    "enabled": snapshot.facades.enabled_count(),
                    "diagnostics": snapshot.facades.diagnostics(),
                },
                "caseProjections": {
                    "catalogEntries": snapshot.case_projections.catalog_entry_count(),
                    "enabled": snapshot.case_projections.enabled_count(),
                    "diagnostics": snapshot.case_projections.diagnostics(),
                },
                "files": inventory,
            }))
            .map_err(|error| EngineResourceError::new("engine_resource_io", error.to_string()))?,
        )?;
        std::fs::rename(&staging, &destination).map_err(EngineResourceError::io)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(error);
    }
    verify_published_snapshot(&destination, &snapshot.id, &inventory)?;
    rebase_snapshot(&mut snapshot, &destination)?;
    Ok((snapshot, destination))
}

fn generated_snapshot_files(
    snapshot: &EngineResourceSnapshot,
) -> Result<BTreeMap<String, Vec<u8>>, EngineResourceError> {
    let mut files = BTreeMap::new();
    for (name, bytes) in &snapshot.catalog_bytes {
        files.insert(format!("catalog/{name}"), bytes.clone());
    }
    for (relative, bytes) in &snapshot.documents {
        let relative = relative.strip_prefix("docs/").unwrap_or(relative);
        files.insert(format!("docs/{relative}"), bytes.clone());
    }
    files.insert(
        "docs-index.json".to_string(),
        serde_json::to_vec_pretty(&snapshot.knowledge.sections)
            .map_err(|error| EngineResourceError::new("engine_resource_io", error.to_string()))?,
    );
    files.insert(
        "examples.json".to_string(),
        serde_json::to_vec_pretty(&snapshot.examples)
            .map_err(|error| EngineResourceError::new("engine_resource_io", error.to_string()))?,
    );
    Ok(files)
}

fn write_published_file(
    root: &Path,
    relative: &str,
    bytes: &[u8],
) -> Result<(), EngineResourceError> {
    let path = published_path(root, relative)?;
    std::fs::write(path, bytes).map_err(EngineResourceError::io)
}

fn published_path(root: &Path, relative: &str) -> Result<PathBuf, EngineResourceError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(EngineResourceError::new(
            "engine_resource_path_escape",
            "invalid published resource path",
        ));
    }
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(EngineResourceError::io)?;
    }
    Ok(path)
}

fn copy_verified_published_file(
    root: &Path,
    relative: &str,
    source: &Path,
    expected_hash: &str,
) -> Result<(), EngineResourceError> {
    let destination = published_path(root, relative)?;
    let mut input = std::fs::File::open(source).map_err(|_| {
        EngineResourceError::new(
            "engine_resource_changed_during_scan",
            format!("resource unreadable: {relative}"),
        )
    })?;
    let mut output = std::fs::File::create(destination).map_err(EngineResourceError::io)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer).map_err(|_| {
            EngineResourceError::new(
                "engine_resource_changed_during_scan",
                format!("resource unreadable: {relative}"),
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut output, &buffer[..read]).map_err(EngineResourceError::io)?;
    }
    if hex::encode(hasher.finalize()) != expected_hash {
        return Err(EngineResourceError::new(
            "engine_resource_changed_during_scan",
            format!("resource changed: {relative}"),
        ));
    }
    Ok(())
}

fn verify_published_snapshot(
    destination: &Path,
    snapshot_id: &str,
    expected: &BTreeMap<String, String>,
) -> Result<(), EngineResourceError> {
    let manifest =
        std::fs::read(safe_file(destination, "manifest.json")?).map_err(EngineResourceError::io)?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest).map_err(|error| {
        EngineResourceError::new("engine_resource_hash_mismatch", error.to_string())
    })?;
    if manifest
        .get("snapshotId")
        .and_then(serde_json::Value::as_str)
        != Some(snapshot_id)
    {
        return Err(EngineResourceError::new(
            "engine_resource_hash_mismatch",
            "published snapshot id mismatch",
        ));
    }
    let declared: BTreeMap<String, String> =
        serde_json::from_value(manifest.get("files").cloned().ok_or_else(|| {
            EngineResourceError::new(
                "engine_resource_hash_mismatch",
                "published file inventory missing",
            )
        })?)
        .map_err(|error| {
            EngineResourceError::new("engine_resource_hash_mismatch", error.to_string())
        })?;
    if &declared != expected {
        return Err(EngineResourceError::new(
            "engine_resource_hash_mismatch",
            "published file inventory mismatch",
        ));
    }
    for (relative, expected_hash) in expected {
        let path = safe_file(destination, relative).map_err(|_| {
            EngineResourceError::new(
                "engine_resource_hash_mismatch",
                format!("published file missing: {relative}"),
            )
        })?;
        if hash_file(&path)? != *expected_hash {
            return Err(EngineResourceError::new(
                "engine_resource_hash_mismatch",
                format!("published file changed: {relative}"),
            ));
        }
    }
    Ok(())
}

fn rebase_snapshot(
    snapshot: &mut EngineResourceSnapshot,
    destination: &Path,
) -> Result<(), EngineResourceError> {
    snapshot.engine_root = safe_directory(destination, "source")?;
    snapshot.source_root = safe_directory(&snapshot.engine_root, "dss")?;
    Ok(())
}
