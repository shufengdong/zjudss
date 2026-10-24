use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zjudss::engine_resources::{
    EngineResourceRegistry, ResourceLimits, SUPPORTED_CATALOG_SCHEMA_VERSIONS,
};

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct Fixture {
    _root: tempfile::TempDir,
    cwd: PathBuf,
}
impl Fixture {
    fn dss(&self) -> PathBuf {
        self.cwd.join("sparrowzz/dss")
    }
    fn bundle_root(&self) -> PathBuf {
        self.dss().join("rssbundle/Grid/123Bus/demo.rssbundle")
    }
    fn load(&self) -> zjudss::engine_resources::EngineResourceSnapshot {
        EngineResourceRegistry::load(&self.cwd, &HashSet::new()).unwrap()
    }
}

fn valid_manifest(main: &[u8], model: &[u8]) -> Value {
    json!({
        "schema_version": 1, "artifact": "mems.dss-rss-bundle",
        "generator": "engine-resource-test/1.0", "psa_contract_version": "0.1.0",
        "entry": "main.rss",
        "study": {"kind":"execution_plan","compatibility":"opendss-native","solver":"radial","results":["voltage"]},
        "source": {"entry":"dss/model.dss","root":"dss","tree_sha256":hash(model)},
        "files": [
            {"path":"dss/model.dss","role":"dss-entry","sha256":hash(model)},
            {"path":"main.rss","role":"main-rss","sha256":hash(main)}
        ],
        "inputs": []
    })
}

fn write_case(bundle: &Path, manifest: &Value) {
    std::fs::create_dir_all(bundle.join("dss")).unwrap();
    std::fs::write(
        bundle.join("main.rss"),
        b"model = import_dss(\"dss/model.dss\");\n",
    )
    .unwrap();
    std::fs::write(
        bundle.join("dss/model.dss"),
        b"new circuit.demo basekv=12.47\n",
    )
    .unwrap();
    std::fs::write(
        bundle.join("manifest.json"),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("engine");
    let dss = cwd.join("sparrowzz/dss");
    std::fs::create_dir_all(dss.join("docs/catalog")).unwrap();
    std::fs::create_dir_all(dss.join("docs/nested")).unwrap();
    std::fs::write(dss.join("docs/catalog/dataframes.json"), br#"{"schema_version":2,"frames":{"load":{"required":{"name":"str","bus":"str"},"optional":{"model":"str"}}}}"#).unwrap();
    std::fs::write(
        dss.join("docs/catalog/functions.json"),
        br#"{"schema_version":2,"frame_functions":[]}"#,
    )
    .unwrap();
    std::fs::write(
        dss.join("docs/nested/guide.md"),
        b"# Guide\nintro\n## Power flow\nUse the validated solver.\n",
    )
    .unwrap();
    std::fs::write(dss.join("demo.rss"), b"result = 1;\n").unwrap();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.demo basekv=12.47\n";
    write_case(
        &dss.join("rssbundle/Grid/123Bus/demo.rssbundle"),
        &valid_manifest(main, model),
    );
    Fixture { _root: root, cwd }
}

#[test]
fn discovers_current_tree_deterministically_and_snapshot_changes_with_loaded_bytes() {
    assert_eq!(SUPPORTED_CATALOG_SCHEMA_VERSIONS, &[2]);
    let fixture = fixture();
    let first = fixture.load();
    assert_eq!(first.id, fixture.load().id);
    assert_eq!(first.examples.len(), 1);
    let example = &first.examples[0];
    assert_eq!(example.category_path, ["Grid", "123Bus"]);
    assert_eq!(example.bundle_path, "Grid/123Bus/demo.rssbundle");
    assert_eq!(example.entry_path.as_deref(), Some("main.rss"));
    let manifest = example.manifest.as_ref().unwrap();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.artifact, "mems.dss-rss-bundle");
    assert_eq!(manifest.generator, "engine-resource-test/1.0");
    assert_eq!(manifest.psa_contract_version, "0.1.0");
    assert_eq!(manifest.study.kind, "execution_plan");
    assert_eq!(manifest.study.compatibility, "opendss-native");
    assert_eq!(manifest.study.solver.as_deref(), Some("radial"));
    assert_eq!(manifest.study.results, ["voltage"]);
    assert_eq!(manifest.source.entry, "dss/model.dss");
    assert_eq!(manifest.source.root, "dss");
    assert_eq!(manifest.inputs, json!([]));
    assert_eq!(
        first.documents.keys().cloned().collect::<Vec<_>>(),
        ["docs/nested/guide.md"]
    );
    assert_eq!(first.knowledge.search("validated solver", 100).len(), 1);
    std::fs::write(
        fixture.dss().join("docs/nested/guide.md"),
        b"# Guide\nchanged\n",
    )
    .unwrap();
    assert_ne!(first.id, fixture.load().id);
}

#[test]
fn discovers_only_manifest_backed_rssbundle_leaves() {
    let fixture = fixture();
    std::fs::create_dir_all(fixture.dss().join("rssbundle/Empty/NoManifest.rssbundle")).unwrap();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.inner basekv=4.16\n";
    write_case(
        &fixture.bundle_root().join("nested/inner.rssbundle"),
        &valid_manifest(main, model),
    );
    std::fs::write(fixture.dss().join("outside.rss"), "result = 2;").unwrap();
    let snapshot = fixture.load();
    assert_eq!(snapshot.examples.len(), 1);
    assert_eq!(snapshot.examples[0].title, "demo");
}

#[test]
fn supported_manifest_unknown_fields_are_ignored_but_contract_mismatches_are_invalid() {
    let fixture = fixture();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.demo basekv=12.47\n";
    let root = fixture.dss().join("rssbundle/Contracts");
    let mut future = valid_manifest(main, model);
    future["future_extension"] = json!({"accepted":true});
    write_case(&root.join("future.rssbundle"), &future);
    let mut schema = valid_manifest(main, model);
    schema["schema_version"] = json!(2);
    write_case(&root.join("schema.rssbundle"), &schema);
    let mut artifact = valid_manifest(main, model);
    artifact["artifact"] = json!("other.bundle");
    write_case(&root.join("artifact.rssbundle"), &artifact);
    let snapshot = fixture.load();
    assert!(
        snapshot
            .examples
            .iter()
            .find(|item| item.title == "future")
            .unwrap()
            .importable
    );
    for title in ["schema", "artifact"] {
        let item = snapshot
            .examples
            .iter()
            .find(|item| item.title == title)
            .unwrap();
        assert!(!item.importable);
        assert!(item.manifest.is_none());
        assert!(item.entry_path.is_none());
    }
}

#[test]
fn bundle_manifest_rejects_duplicate_missing_unsafe_and_hash_mismatch_files() {
    let fixture = fixture();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.demo basekv=12.47\n";
    let root = fixture.dss().join("rssbundle/Invalid");
    let mut duplicate = valid_manifest(main, model);
    duplicate["files"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"main.rss","role":"copy","sha256":hash(main)}));
    write_case(&root.join("duplicate.rssbundle"), &duplicate);
    let mut missing = valid_manifest(main, model);
    missing["files"]
        .as_array_mut()
        .unwrap()
        .push(json!({"path":"missing.dss","role":"dss-source","sha256":hash(b"missing")}));
    write_case(&root.join("missing.rssbundle"), &missing);
    let mut unsafe_manifest = valid_manifest(main, model);
    unsafe_manifest["entry"] = json!("/home/operator/private/main.rss");
    write_case(&root.join("unsafe.rssbundle"), &unsafe_manifest);
    let mismatch = root.join("mismatch.rssbundle");
    write_case(&mismatch, &valid_manifest(main, model));
    std::fs::write(mismatch.join("dss/model.dss"), "changed").unwrap();
    let snapshot = fixture.load();
    for title in ["duplicate", "missing", "unsafe", "mismatch"] {
        let item = snapshot
            .examples
            .iter()
            .find(|item| item.title == title)
            .unwrap();
        assert!(!item.importable, "{title}: {:?}", item.diagnostics);
        assert!(!item.diagnostics.is_empty());
    }
    let serialized = serde_json::to_string(&snapshot.examples).unwrap();
    assert!(!serialized.contains("/home/operator"));
    assert!(!serialized.contains(fixture.cwd.to_string_lossy().as_ref()));
}

#[test]
fn bundle_manifest_without_a_dss_source_is_not_importable() {
    let fixture = fixture();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.demo basekv=12.47\n";
    let mut manifest = valid_manifest(main, model);
    manifest["files"] = json!([
        {"path":"main.rss","role":"main-rss","sha256":hash(main)}
    ]);
    let bundle = fixture.dss().join("rssbundle/Invalid/no-dss.rssbundle");
    write_case(&bundle, &manifest);

    let snapshot = fixture.load();
    let example = snapshot
        .examples
        .iter()
        .find(|example| example.title == "no-dss")
        .unwrap();

    assert!(!example.importable);
    assert_eq!(example.diagnostics, ["model_import_source_missing"]);
}

#[cfg(unix)]
#[test]
fn bundle_manifest_rejects_symlinked_files_without_blocking_other_cases() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.demo basekv=12.47\n";
    let bundle = fixture.dss().join("rssbundle/Invalid/symlink.rssbundle");
    write_case(&bundle, &valid_manifest(main, model));
    std::fs::remove_file(bundle.join("dss/model.dss")).unwrap();
    symlink(fixture.dss().join("demo.rss"), bundle.join("dss/model.dss")).unwrap();
    let snapshot = fixture.load();
    assert!(
        !snapshot
            .examples
            .iter()
            .find(|item| item.title == "symlink")
            .unwrap()
            .importable
    );
    assert!(
        snapshot
            .examples
            .iter()
            .any(|item| item.title == "demo" && item.importable)
    );
}

#[test]
fn publish_freezes_only_manifest_declared_case_sources_and_rebases_the_snapshot() {
    let fixture = fixture();
    let data = tempfile::tempdir().unwrap();
    let (published, path) = EngineResourceRegistry::publish(fixture.load(), data.path()).unwrap();
    assert_eq!(published.engine_root, path.join("source"));
    assert!(path.join("catalog/dataframes.json").is_file());
    assert!(path.join("docs/nested/guide.md").is_file());
    assert!(!path.join("source/sparrowzz/dss/demo.rss").exists());
    let bundle = path.join("source/sparrowzz/dss/rssbundle/Grid/123Bus/demo.rssbundle");
    assert!(bundle.join("manifest.json").is_file());
    assert!(bundle.join("main.rss").is_file());
    assert!(bundle.join("dss/model.dss").is_file());
}

#[test]
fn publish_rejects_source_changes_and_corrupt_existing_snapshots() {
    let changed = fixture();
    let snapshot = changed.load();
    std::fs::write(changed.bundle_root().join("dss/model.dss"), "changed").unwrap();
    assert_eq!(
        EngineResourceRegistry::publish(snapshot, tempfile::tempdir().unwrap().path())
            .unwrap_err()
            .code(),
        "engine_resource_changed_during_scan"
    );
    let fixture = fixture();
    let snapshot = fixture.load();
    let data = tempfile::tempdir().unwrap();
    let (_, path) = EngineResourceRegistry::publish(snapshot.clone(), data.path()).unwrap();
    std::fs::write(
        path.join("source/sparrowzz/dss/rssbundle/Grid/123Bus/demo.rssbundle/dss/model.dss"),
        "corrupt",
    )
    .unwrap();
    assert_eq!(
        EngineResourceRegistry::publish(snapshot, data.path())
            .unwrap_err()
            .code(),
        "engine_resource_hash_mismatch"
    );
}

#[test]
fn validates_catalog_schema_and_required_shape_but_allows_no_examples() {
    let empty = fixture();
    std::fs::remove_dir_all(empty.dss().join("rssbundle")).unwrap();
    assert!(empty.load().examples.is_empty());
    let unsupported = fixture();
    std::fs::write(
        unsupported.dss().join("docs/catalog/dataframes.json"),
        br#"{"schema_version":3,"frames":{"load":{"required":{"name":"str"}}}}"#,
    )
    .unwrap();
    assert_eq!(
        EngineResourceRegistry::load(&unsupported.cwd, &HashSet::new())
            .unwrap_err()
            .code(),
        "engine_contract_mismatch"
    );
    let malformed = fixture();
    std::fs::write(
        malformed.dss().join("docs/catalog/functions.json"),
        br#"{"schema_version":2}"#,
    )
    .unwrap();
    assert_eq!(
        EngineResourceRegistry::load(&malformed.cwd, &HashSet::new())
            .unwrap_err()
            .code(),
        "catalog_invalid"
    );
}

#[test]
fn malformed_manifest_is_an_isolated_invalid_leaf() {
    let fixture = fixture();
    let malformed = fixture.dss().join("rssbundle/Invalid/malformed.rssbundle");
    std::fs::create_dir_all(&malformed).unwrap();
    std::fs::write(malformed.join("manifest.json"), b"{").unwrap();
    let snapshot = fixture.load();
    let item = snapshot
        .examples
        .iter()
        .find(|item| item.title == "malformed")
        .unwrap();
    assert!(!item.importable);
    assert!(item.manifest.is_none());
    assert!(item.entry_path.is_none());
    assert!(
        snapshot
            .examples
            .iter()
            .any(|item| item.title == "demo" && item.importable)
    );
}

#[test]
fn all_resource_limits_fail_with_one_stable_code() {
    let cases: Vec<Box<dyn Fn(&Fixture) -> ResourceLimits>> = vec![
        Box::new(|fixture| {
            std::fs::write(fixture.dss().join("docs/second.md"), "second").unwrap();
            ResourceLimits {
                max_documents: 1,
                ..ResourceLimits::default()
            }
        }),
        Box::new(|_| ResourceLimits {
            max_document_bytes: 1,
            ..ResourceLimits::default()
        }),
        Box::new(|_| ResourceLimits {
            max_total_document_bytes: 1,
            ..ResourceLimits::default()
        }),
        Box::new(|fixture| {
            let main = b"model = import_dss(\"dss/model.dss\");\n";
            let model = b"new circuit.second basekv=4.16\n";
            write_case(
                &fixture.dss().join("rssbundle/second.rssbundle"),
                &valid_manifest(main, model),
            );
            ResourceLimits {
                max_examples: 1,
                ..ResourceLimits::default()
            }
        }),
        Box::new(|_| ResourceLimits {
            max_example_files: 2,
            ..ResourceLimits::default()
        }),
        Box::new(|_| ResourceLimits {
            max_example_bytes: 1,
            ..ResourceLimits::default()
        }),
    ];
    for limits in cases {
        let fixture = fixture();
        assert_eq!(
            EngineResourceRegistry::load_with_limits(
                &fixture.cwd,
                &HashSet::new(),
                limits(&fixture)
            )
            .unwrap_err()
            .code(),
            "engine_resource_limit_exceeded"
        );
    }
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_anywhere_in_global_document_tree() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let outside = fixture.cwd.join("outside.md");
    std::fs::write(&outside, "outside").unwrap();
    symlink(&outside, fixture.dss().join("docs/linked.md")).unwrap();
    assert_eq!(
        EngineResourceRegistry::load(&fixture.cwd, &HashSet::new())
            .unwrap_err()
            .code(),
        "engine_resource_path_escape"
    );
}

#[test]
fn publish_normalizes_lexical_parent_components_in_data_dir() {
    let fixture = fixture();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("nested")).unwrap();
    let (_, published) =
        EngineResourceRegistry::publish(fixture.load(), &data.path().join("nested/../data"))
            .unwrap();
    assert!(published.is_absolute());
    assert!(!published.to_string_lossy().contains("/../"));
    assert!(published.join("manifest.json").is_file());
}

#[cfg(unix)]
#[test]
fn publish_accepts_a_symlinked_data_dir_root() {
    use std::os::unix::fs::symlink;
    let fixture = fixture();
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real-data");
    std::fs::create_dir_all(&real).unwrap();
    let linked = root.path().join("linked-data");
    symlink(&real, &linked).unwrap();
    let (_, published) = EngineResourceRegistry::publish(fixture.load(), &linked).unwrap();
    assert!(published.starts_with(real.canonicalize().unwrap()));
}

#[test]
fn current_eig_rs_tree_is_accepted_without_commit_hash_or_exact_example_count() {
    let path = PathBuf::from("../../eig-rs");
    if path
        .join("sparrowzz/dss/docs/catalog/dataframes.json")
        .is_file()
    {
        let snapshot = EngineResourceRegistry::load(&path, &HashSet::new())
            .expect("current configured eig-rs resources must be dynamically valid");
        assert_eq!(snapshot.catalog_schema_version, 2);
        assert!(
            snapshot
                .examples
                .iter()
                .all(|item| item.source_kind == "rssbundle")
        );
    }
}
