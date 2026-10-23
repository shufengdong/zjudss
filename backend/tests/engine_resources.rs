use std::collections::HashSet;
use std::path::PathBuf;

use backend::engine_resources::{
    EngineResourceRegistry, ResourceLimits, SUPPORTED_CATALOG_SCHEMA_VERSIONS,
};
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Fixture {
    _root: tempfile::TempDir,
    cwd: PathBuf,
}

impl Fixture {
    fn dss(&self) -> PathBuf {
        self.cwd.join("sparrowzz/dss")
    }

    fn load(&self) -> backend::engine_resources::EngineResourceSnapshot {
        EngineResourceRegistry::load(&self.cwd, &HashSet::new()).unwrap()
    }
}

fn fixture(required_feature: Option<&str>) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("engine");
    let dss = cwd.join("sparrowzz/dss");
    std::fs::create_dir_all(dss.join("docs/catalog")).unwrap();
    std::fs::create_dir_all(dss.join("docs/nested")).unwrap();
    std::fs::create_dir_all(dss.join("rssbundle/demo.rssbundle")).unwrap();
    std::fs::write(
        dss.join("docs/catalog/dataframes.json"),
        br#"{"schema_version":2,"frames":{"load":{"required":{"name":"str","bus":"str"},"optional":{"model":"str"}}}}"#,
    )
    .unwrap();
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
    let main = b"result = 1;\n";
    std::fs::write(dss.join("rssbundle/demo.rssbundle/main.rss"), main).unwrap();
    let required: Vec<&str> = required_feature.into_iter().collect();
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "entry":"main.rss",
        "files":[{"path":"main.rss","sha256":hash(main)}],
        "required_features":required,
        "study":{"kind":"power_flow","solver":"radial"}
    }))
    .unwrap();
    std::fs::write(dss.join("rssbundle/demo.rssbundle/manifest.json"), manifest).unwrap();
    Fixture { _root: root, cwd }
}

#[test]
fn discovers_current_tree_deterministically_and_snapshot_changes_with_loaded_bytes() {
    assert_eq!(SUPPORTED_CATALOG_SCHEMA_VERSIONS, &[2]);
    let fixture = fixture(None);
    let first = fixture.load();
    let second = fixture.load();
    assert_eq!(first.id, second.id);
    assert_eq!(first.examples.len(), 2);
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
fn publish_freezes_every_loaded_source_and_rebases_the_snapshot() {
    let fixture = fixture(None);
    let snapshot = fixture.load();
    let data = tempfile::tempdir().unwrap();
    let (published, path) = EngineResourceRegistry::publish(snapshot, data.path()).unwrap();
    assert_eq!(published.engine_root, path.join("source"));
    assert!(path.join("catalog/dataframes.json").is_file());
    assert!(path.join("docs/nested/guide.md").is_file());
    assert!(path.join("source/sparrowzz/dss/demo.rss").is_file());
    assert!(
        path.join("source/sparrowzz/dss/rssbundle/demo.rssbundle/manifest.json")
            .is_file()
    );
    assert!(
        path.join("source/sparrowzz/dss/rssbundle/demo.rssbundle/main.rss")
            .is_file()
    );
}

#[test]
fn publish_rejects_source_changes_and_corrupt_existing_snapshots() {
    let changed = fixture(None);
    let snapshot = changed.load();
    std::fs::write(changed.dss().join("demo.rss"), "changed").unwrap();
    assert_eq!(
        EngineResourceRegistry::publish(snapshot, tempfile::tempdir().unwrap().path())
            .unwrap_err()
            .code(),
        "engine_resource_changed_during_scan"
    );

    let fixture = fixture(None);
    let snapshot = fixture.load();
    let data = tempfile::tempdir().unwrap();
    let (_, path) = EngineResourceRegistry::publish(snapshot.clone(), data.path()).unwrap();
    std::fs::write(path.join("source/sparrowzz/dss/demo.rss"), "corrupt").unwrap();
    assert_eq!(
        EngineResourceRegistry::publish(snapshot, data.path())
            .unwrap_err()
            .code(),
        "engine_resource_hash_mismatch"
    );
}

#[test]
fn validates_catalog_schema_and_required_shape_but_allows_no_examples() {
    let empty = fixture(None);
    std::fs::remove_file(empty.dss().join("demo.rss")).unwrap();
    std::fs::remove_dir_all(empty.dss().join("rssbundle")).unwrap();
    assert!(empty.load().examples.is_empty());

    let unsupported = fixture(None);
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

    let malformed = fixture(None);
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
fn individual_dependency_bundle_hash_and_feature_failures_are_non_importable() {
    let broken = fixture(Some("mumps"));
    std::fs::write(
        broken.dss().join("demo.rss"),
        b"model = import_dss(choose_path());\n",
    )
    .unwrap();
    std::fs::write(
        broken.dss().join("rssbundle/demo.rssbundle/main.rss"),
        b"changed",
    )
    .unwrap();
    let snapshot = broken.load();
    let rss = snapshot
        .examples
        .iter()
        .find(|example| example.source_kind == "rss")
        .unwrap();
    assert!(!rss.importable);
    assert!(
        rss.diagnostics
            .iter()
            .any(|item| item == "dynamic dependency")
    );
    let bundle = snapshot
        .examples
        .iter()
        .find(|example| example.source_kind == "rssbundle")
        .unwrap();
    assert!(!bundle.importable);
    assert!(
        bundle
            .diagnostics
            .iter()
            .any(|item| item == "hash mismatch: main.rss")
    );
    assert!(
        bundle
            .diagnostics
            .iter()
            .any(|item| item == "missing feature: mumps")
    );
    assert!(
        snapshot
            .examples
            .iter()
            .flat_map(|example| &example.diagnostics)
            .all(|diagnostic| !diagnostic.contains(broken.cwd.to_string_lossy().as_ref()))
    );

    let capable = fixture(Some("mumps"));
    let snapshot =
        EngineResourceRegistry::load(&capable.cwd, &HashSet::from(["mumps".to_string()])).unwrap();
    assert!(
        snapshot
            .examples
            .iter()
            .find(|example| example.source_kind == "rssbundle")
            .unwrap()
            .importable
    );
}

#[test]
fn malformed_and_unsafe_examples_are_isolated_with_redacted_diagnostics() {
    let fixture = fixture(None);
    std::fs::write(
        fixture.dss().join("demo.rss"),
        b"model = import_dss(\"/home/operator/private/model.dss\");\n",
    )
    .unwrap();

    let malformed = fixture.dss().join("rssbundle/malformed.rssbundle");
    std::fs::create_dir_all(&malformed).unwrap();
    std::fs::write(malformed.join("manifest.json"), b"{").unwrap();

    let unsafe_bundle = fixture.dss().join("rssbundle/unsafe.rssbundle");
    std::fs::create_dir_all(&unsafe_bundle).unwrap();
    std::fs::write(
        unsafe_bundle.join("manifest.json"),
        serde_json::to_vec(&serde_json::json!({
            "entry":"/home/operator/private/main.rss",
            "files":[{"path":"/home/operator/private/model.dss","sha256":"bad"}]
        }))
        .unwrap(),
    )
    .unwrap();

    let snapshot = fixture.load();
    for name in ["malformed", "unsafe"] {
        let example = snapshot
            .examples
            .iter()
            .find(|example| example.title == name)
            .unwrap();
        assert!(!example.importable);
        assert!(!example.diagnostics.is_empty());
    }
    assert!(snapshot.examples.iter().any(|example| example.importable));
    let serialized = serde_json::to_string(&snapshot.examples).unwrap();
    assert!(!serialized.contains("/home/operator"));
    assert!(!serialized.contains(fixture.cwd.to_string_lossy().as_ref()));
    assert!(
        snapshot
            .examples
            .iter()
            .flat_map(|example| &example.diagnostics)
            .all(|diagnostic| {
                !diagnostic.contains("/home/operator")
                    && !diagnostic.contains(fixture.cwd.to_string_lossy().as_ref())
            })
    );
}

#[test]
fn duplicate_normalized_example_ids_fail_startup() {
    let fixture = fixture(None);
    std::fs::write(fixture.dss().join("DEMO.rss"), b"result = 2;\n").unwrap();
    assert_eq!(
        EngineResourceRegistry::load(&fixture.cwd, &HashSet::new())
            .unwrap_err()
            .code(),
        "duplicate_example_id"
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
        Box::new(|_| ResourceLimits {
            max_examples: 1,
            ..ResourceLimits::default()
        }),
        Box::new(|_| ResourceLimits {
            max_example_files: 1,
            ..ResourceLimits::default()
        }),
        Box::new(|_| ResourceLimits {
            max_example_bytes: 1,
            ..ResourceLimits::default()
        }),
        Box::new(|fixture| {
            std::fs::create_dir_all(fixture.dss().join("models")).unwrap();
            std::fs::write(
                fixture.dss().join("demo.rss"),
                "import_dss(\"models/a.dss\")",
            )
            .unwrap();
            std::fs::write(fixture.dss().join("models/a.dss"), "redirect b.dss").unwrap();
            std::fs::write(fixture.dss().join("models/b.dss"), "new circuit.x").unwrap();
            ResourceLimits {
                max_dependency_depth: 1,
                ..ResourceLimits::default()
            }
        }),
    ];
    for limits in cases {
        let fixture = fixture(None);
        let limits = limits(&fixture);
        assert_eq!(
            EngineResourceRegistry::load_with_limits(&fixture.cwd, &HashSet::new(), limits)
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

    let fixture = fixture(None);
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
    let fixture = fixture(None);
    let snapshot = fixture.load();
    let data = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("nested")).unwrap();
    let lexical = data.path().join("nested/../data");
    let (_, published) = EngineResourceRegistry::publish(snapshot, &lexical).unwrap();
    assert!(published.is_absolute());
    assert!(!published.to_string_lossy().contains("/../"));
    assert!(published.join("manifest.json").is_file());
}

#[cfg(unix)]
#[test]
fn publish_accepts_a_symlinked_data_dir_root() {
    use std::os::unix::fs::symlink;

    let fixture = fixture(None);
    let snapshot = fixture.load();
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real-data");
    std::fs::create_dir_all(&real).unwrap();
    let linked = root.path().join("linked-data");
    symlink(&real, &linked).unwrap();
    let (_, published) = EngineResourceRegistry::publish(snapshot, &linked).unwrap();
    assert!(published.starts_with(real.canonicalize().unwrap()));
    assert!(published.join("manifest.json").is_file());
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
    }
}
