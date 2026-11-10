//! 从当前资源树加载 catalog、facade、projection、文档和案例并构造快照。

use std::collections::BTreeMap;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::tree::{
    discover_bundle_manifests, discover_documents, hash_bytes, limit_error, portable_relative,
    read_bounded_file, read_safe_bounded, safe_directory,
};
use super::{
    CaseProjectionRegistry, CatalogSchemaVersions, EngineKnowledge, EngineResourceError,
    EngineResourceSnapshot, FacadeRegistry, ResourceLimits,
    SUPPORTED_CASE_PROJECTIONS_SCHEMA_VERSION, SUPPORTED_DATAFRAMES_SCHEMA_VERSION,
    SUPPORTED_DTYPE_GRAMMAR_VERSION, SUPPORTED_FUNCTIONS_SCHEMA_VERSION,
    SUPPORTED_STUDY_FACADES_SCHEMA_VERSION, examples,
};
use crate::adapters::ENGINE_CONTRACT_VERSION;
use crate::catalog::SchemaRegistry;

pub(super) fn load_snapshot(
    sparrowzz_dir: &Path,
    limits: ResourceLimits,
) -> Result<EngineResourceSnapshot, EngineResourceError> {
    let engine_root = sparrowzz_dir
        .canonicalize()
        .map_err(EngineResourceError::io)?;
    let dss_root = safe_directory(&engine_root, "dss")?;
    let dataframes = read_safe_bounded(
        &dss_root,
        "docs/catalog/dataframes.json",
        limits.max_document_bytes,
    )?;
    let functions = read_safe_bounded(
        &dss_root,
        "docs/catalog/functions.json",
        limits.max_document_bytes,
    )?;
    let study_facades_path = dss_root.join("docs/catalog/study_facades.json");
    let study_facades_missing = matches!(
        std::fs::symlink_metadata(&study_facades_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    );
    let study_facades = if study_facades_missing {
        br#"{"schema_version":1,"facades":[]}"#.to_vec()
    } else {
        read_safe_bounded(
            &dss_root,
            "docs/catalog/study_facades.json",
            limits.max_document_bytes,
        )?
    };
    let case_projections = read_safe_bounded(
        &dss_root,
        "docs/catalog/case_projections.json",
        limits.max_document_bytes,
    )?;
    let catalog_json: serde_json::Value = serde_json::from_slice(&dataframes)
        .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
    let functions_json: serde_json::Value = serde_json::from_slice(&functions)
        .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
    let study_facades_json: serde_json::Value = serde_json::from_slice(&study_facades)
        .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
    let case_projections_json: serde_json::Value = serde_json::from_slice(&case_projections)
        .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
    let catalog_schema_versions = CatalogSchemaVersions {
        functions: supported_schema_version(
            &functions_json,
            "functions",
            SUPPORTED_FUNCTIONS_SCHEMA_VERSION,
        )?,
        study_facades: supported_schema_version(
            &study_facades_json,
            "study_facades",
            SUPPORTED_STUDY_FACADES_SCHEMA_VERSION,
        )?,
        case_projections: supported_schema_version(
            &case_projections_json,
            "case_projections",
            SUPPORTED_CASE_PROJECTIONS_SCHEMA_VERSION,
        )?,
        dataframes: supported_schema_version(
            &catalog_json,
            "dataframes",
            SUPPORTED_DATAFRAMES_SCHEMA_VERSION,
        )?,
        dtype_grammar: supported_nested_version(
            &catalog_json,
            &["dtype_grammar", "version"],
            "dtype grammar",
            SUPPORTED_DTYPE_GRAMMAR_VERSION,
        )?,
    };
    validate_catalog_shape(&catalog_json, &functions_json, &study_facades_json)?;
    let mut schemas = SchemaRegistry::from_catalog(&catalog_json)
        .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?;
    let facades = if study_facades_missing {
        FacadeRegistry::unavailable(
            "study_facades_missing",
            "当前引擎 snapshot 缺少 study_facades.json，没有可用研究门面",
        )
    } else {
        FacadeRegistry::from_catalogs(&study_facades_json, &mut schemas)
            .map_err(|error| EngineResourceError::new("catalog_invalid", error.to_string()))?
    };
    let case_projection_registry =
        CaseProjectionRegistry::from_catalogs(&case_projections_json, &facades, &mut schemas)?;

    let mut docs = Vec::new();
    let mut documents = BTreeMap::new();
    let mut total_document_bytes = 0_u64;
    for path in discover_documents(&dss_root, limits.max_documents)? {
        let relative = portable_relative(&dss_root, &path)?;
        let remaining = limits
            .max_total_document_bytes
            .checked_sub(total_document_bytes)
            .ok_or_else(|| limit_error("documentation total bytes"))?;
        let bytes = read_bounded_file(
            &path,
            limits.max_document_bytes.min(remaining),
            "documentation file or total size",
        )?;
        total_document_bytes = total_document_bytes.saturating_add(bytes.len() as u64);
        if total_document_bytes > limits.max_total_document_bytes {
            return Err(limit_error("documentation total bytes"));
        }
        let content = String::from_utf8(bytes).map_err(|error| {
            EngineResourceError::new("engine_doc_invalid_utf8", error.to_string())
        })?;
        documents.insert(relative.clone(), content.as_bytes().to_vec());
        docs.push((relative, content));
    }
    let bundles =
        discover_bundle_manifests(&dss_root, limits.max_examples, limits.max_example_bytes)?;
    let examples = examples::load_examples(
        &engine_root,
        &dss_root,
        &bundles,
        &case_projection_registry,
        limits,
    )?;
    let knowledge = EngineKnowledge::from_documents(&docs);
    let mut source_hashes = BTreeMap::new();
    insert_source_hash(
        &mut source_hashes,
        "dss/docs/catalog/dataframes.json".to_string(),
        hash_bytes(&dataframes),
    )?;
    insert_source_hash(
        &mut source_hashes,
        "dss/docs/catalog/functions.json".to_string(),
        hash_bytes(&functions),
    )?;
    if !study_facades_missing {
        insert_source_hash(
            &mut source_hashes,
            "dss/docs/catalog/study_facades.json".to_string(),
            hash_bytes(&study_facades),
        )?;
    }
    insert_source_hash(
        &mut source_hashes,
        "dss/docs/catalog/case_projections.json".to_string(),
        hash_bytes(&case_projections),
    )?;
    for (path, bytes) in &documents {
        insert_source_hash(&mut source_hashes, format!("dss/{path}"), hash_bytes(bytes))?;
    }
    for example in &examples {
        for file in &example.files {
            insert_source_hash(
                &mut source_hashes,
                file.source_path.clone(),
                file.sha256.clone(),
            )?;
        }
    }
    let mut identity = Sha256::new();
    identity_part(&mut identity, b"format", b"zjudss-engine-resource-v1");
    identity_part(
        &mut identity,
        b"contract",
        ENGINE_CONTRACT_VERSION.as_bytes(),
    );
    identity_part(
        &mut identity,
        b"study-facades-source",
        if study_facades_missing {
            b"missing"
        } else {
            b"present"
        },
    );
    for (name, version) in [
        ("functions", catalog_schema_versions.functions),
        ("study_facades", catalog_schema_versions.study_facades),
        ("case_projections", catalog_schema_versions.case_projections),
        ("dataframes", catalog_schema_versions.dataframes),
        ("dtype_grammar", catalog_schema_versions.dtype_grammar),
    ] {
        identity_part(
            &mut identity,
            format!("schema-version-{name}").as_bytes(),
            version.to_string().as_bytes(),
        );
    }
    for (path, hash) in &source_hashes {
        identity_part(&mut identity, path.as_bytes(), hash.as_bytes());
    }
    for example in &examples {
        identity_part(&mut identity, b"example-id", example.id.as_bytes());
        identity_part(
            &mut identity,
            b"example-category",
            example.category_path.join("/").as_bytes(),
        );
        identity_part(
            &mut identity,
            b"example-bundle",
            example.bundle_path.as_bytes(),
        );
        identity_part(&mut identity, b"example-tree", example.tree_hash.as_bytes());
        identity_part(
            &mut identity,
            b"example-importable",
            if example.importable { b"1" } else { b"0" },
        );
        for diagnostic in &example.diagnostics {
            identity_part(&mut identity, b"diagnostic", diagnostic.as_bytes());
        }
        if let Some(manifest) = &example.manifest {
            let manifest = serde_json::to_vec(manifest).map_err(|error| {
                EngineResourceError::new("engine_resource_manifest_invalid", error.to_string())
            })?;
            identity_part(&mut identity, b"example-manifest", &manifest);
        }
        for file in &example.files {
            identity_part(
                &mut identity,
                b"example-file-path",
                file.virtual_path.as_bytes(),
            );
            identity_part(&mut identity, b"example-file-role", file.role.as_bytes());
            identity_part(&mut identity, b"example-file-hash", file.sha256.as_bytes());
        }
    }
    let id = format!("ers_{}", &hex::encode(identity.finalize())[..32]);
    Ok(EngineResourceSnapshot {
        id,
        contract_version: ENGINE_CONTRACT_VERSION.to_string(),
        catalog_schema_versions,
        dataframes_sha256: hash_bytes(&dataframes),
        functions_sha256: hash_bytes(&functions),
        study_facades_sha256: hash_bytes(&study_facades),
        case_projections_sha256: hash_bytes(&case_projections),
        source_root: dss_root,
        engine_root,
        catalog_json,
        functions_json,
        study_facades_json,
        case_projections_json,
        schemas,
        facades,
        case_projections: case_projection_registry,
        catalog_bytes: BTreeMap::from([
            ("dataframes.json".to_string(), dataframes),
            ("functions.json".to_string(), functions),
            ("study_facades.json".to_string(), study_facades),
            ("case_projections.json".to_string(), case_projections),
        ]),
        documents,
        knowledge,
        examples,
        source_hashes,
    })
}

fn identity_part(hasher: &mut Sha256, key: &[u8], value: &[u8]) {
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn insert_source_hash(
    hashes: &mut BTreeMap<String, String>,
    path: String,
    hash: String,
) -> Result<(), EngineResourceError> {
    if let Some(existing) = hashes.get(&path) {
        if existing != &hash {
            return Err(EngineResourceError::new(
                "engine_resource_changed_during_scan",
                format!("resource changed while scanning: {path}"),
            ));
        }
        return Ok(());
    }
    hashes.insert(path, hash);
    Ok(())
}

fn supported_schema_version(
    value: &serde_json::Value,
    catalog: &str,
    supported: u32,
) -> Result<u32, EngineResourceError> {
    let schema = value
        .get("schema_version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| EngineResourceError::new("catalog_invalid", "缺少 schema_version"))?;
    if schema != supported {
        return Err(EngineResourceError::new(
            "engine_contract_mismatch",
            format!("不支持 {catalog} schema version {schema}，当前支持 {supported}"),
        ));
    }
    Ok(schema)
}

fn supported_nested_version(
    value: &serde_json::Value,
    path: &[&str],
    catalog: &str,
    supported: u32,
) -> Result<u32, EngineResourceError> {
    let mut current = value;
    for part in path {
        current = current.get(*part).ok_or_else(|| {
            EngineResourceError::new("catalog_invalid", format!("{catalog} 缺少版本字段"))
        })?;
    }
    let version = current
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            EngineResourceError::new("catalog_invalid", format!("{catalog} 版本无效"))
        })?;
    if version != supported {
        return Err(EngineResourceError::new(
            "engine_contract_mismatch",
            format!("不支持 {catalog} version {version}，当前支持 {supported}"),
        ));
    }
    Ok(version)
}

fn validate_catalog_shape(
    dataframes: &serde_json::Value,
    functions: &serde_json::Value,
    study_facades: &serde_json::Value,
) -> Result<(), EngineResourceError> {
    let frames = dataframes
        .get("frames")
        .and_then(serde_json::Value::as_object);
    let analysis = dataframes
        .get("analysis_inputs")
        .and_then(serde_json::Value::as_object);
    if frames.is_none_or(serde_json::Map::is_empty)
        && analysis.is_none_or(serde_json::Map::is_empty)
    {
        return Err(EngineResourceError::new(
            "catalog_invalid",
            "dataframes catalog 缺少 frames/analysis_inputs",
        ));
    }
    if functions
        .get("frame_functions")
        .and_then(serde_json::Value::as_array)
        .is_none()
    {
        return Err(EngineResourceError::new(
            "catalog_invalid",
            "functions catalog 缺少 frame_functions",
        ));
    }
    if study_facades
        .get("facades")
        .and_then(serde_json::Value::as_array)
        .is_none()
    {
        return Err(EngineResourceError::new(
            "catalog_invalid",
            "study_facades catalog 缺少 facades",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicting_inventory_hashes_fail_as_a_changed_scan() {
        let mut inventory = BTreeMap::new();
        insert_source_hash(&mut inventory, "shared/model.dss".into(), "first".into()).unwrap();
        let error = insert_source_hash(&mut inventory, "shared/model.dss".into(), "second".into())
            .unwrap_err();
        assert_eq!(error.code(), "engine_resource_changed_during_scan");
    }
}
