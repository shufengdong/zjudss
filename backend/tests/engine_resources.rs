use std::collections::HashSet;
use std::path::{Path, PathBuf};

use backend::engine_resources::{EngineResourceRegistry, ResourceContract, SourceFileSpec};
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Fixture {
    _root: tempfile::TempDir,
    cwd: PathBuf,
    contract: ResourceContract,
}

fn fixture(required_feature: Option<&str>) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("engine");
    let dss = cwd.join("sparrowzz/dss");
    std::fs::create_dir_all(dss.join("docs/catalog")).unwrap();
    std::fs::create_dir_all(dss.join("rssbundle/demo.rssbundle")).unwrap();
    let catalog = br#"{"schema_version":2,"frames":{"load":{"required":{"name":"str","bus":"str"},"optional":{"model":"str"}}}}"#;
    let functions = br#"{"schema_version":2,"frame_functions":[]}"#;
    std::fs::write(dss.join("docs/catalog/dataframes.json"), catalog).unwrap();
    std::fs::write(dss.join("docs/catalog/functions.json"), functions).unwrap();
    let doc = b"# Guide\nintro\n## Power flow\nUse the validated solver.\n";
    std::fs::write(dss.join("docs/guide.md"), doc).unwrap();
    let rss = b"x = 1\n";
    std::fs::write(dss.join("demo.rss"), rss).unwrap();
    let main = b"result = 1\n";
    std::fs::write(dss.join("rssbundle/demo.rssbundle/main.rss"), main).unwrap();
    let required: Vec<&str> = required_feature.into_iter().collect();
    let manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "entry":"main.rss", "files":[{"path":"main.rss","sha256":hash(main)}],
        "required_features":required, "study":{"kind":"power_flow","solver":"radial"}
    }))
    .unwrap();
    let manifest_path = dss.join("rssbundle/demo.rssbundle/manifest.json");
    std::fs::write(&manifest_path, &manifest).unwrap();
    let mut docs_hasher = Sha256::new();
    docs_hasher.update(b"docs/guide.md");
    docs_hasher.update(b"\0");
    docs_hasher.update(doc);
    docs_hasher.update(b"\0");
    let contract = ResourceContract {
        contract_version: "fixture-v1".into(),
        catalog_schema_version: 2,
        dataframes_sha256: hash(catalog),
        functions_sha256: hash(functions),
        docs_tree_sha256: format!("{:x}", docs_hasher.finalize()),
        doc_paths: vec!["docs/guide.md".into()],
        root_rss: vec![SourceFileSpec {
            path: "demo.rss".into(),
            sha256: hash(rss),
        }],
        bundles: vec![SourceFileSpec {
            path: "rssbundle/demo.rssbundle/manifest.json".into(),
            sha256: hash(&manifest),
        }],
    };
    Fixture {
        _root: root,
        cwd,
        contract,
    }
}

#[test]
fn embedded_contract_pins_eleven_docs_sixteen_rss_and_eighty_one_bundles() {
    let contract = EngineResourceRegistry::embedded_contract().unwrap();
    assert_eq!(contract.doc_paths.len(), 11);
    assert_eq!(contract.root_rss.len(), 16);
    assert_eq!(contract.bundles.len(), 81);
}

#[test]
fn loads_searchable_stable_snapshot_and_publishes_it_atomically() {
    let fixture = fixture(None);
    let first = EngineResourceRegistry::load_with_contract(
        &fixture.cwd,
        fixture.contract.clone(),
        &HashSet::new(),
    )
    .unwrap();
    let second =
        EngineResourceRegistry::load_with_contract(&fixture.cwd, fixture.contract, &HashSet::new())
            .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.examples.len(), 2);
    assert_eq!(first.knowledge.sections.len(), 2);
    let hits = first.knowledge.search("validated solver", 100);
    assert_eq!(hits.len(), 1);
    let section = first.knowledge.read(&hits[0].section_id, 12).unwrap();
    assert_eq!(section.hash, hits[0].hash);
    assert!(section.content.chars().count() <= 12);
    let data = tempfile::tempdir().unwrap();
    let published = EngineResourceRegistry::publish(&first, data.path()).unwrap();
    assert!(published.join("manifest.json").is_file());
    assert_eq!(
        published,
        EngineResourceRegistry::publish(&first, data.path()).unwrap()
    );
}

#[test]
fn data_dependencies_are_hashed_without_parsing_embedded_schema_urls() {
    let mut fixture = fixture(None);
    let rss = br#"model = import_bmopf("psa/tests/bmopf/model.json");
network = import_dss("sparrowzz/dss/models/network.dss");"#;
    let model = br#"{"$schema":"https://example.invalid/bmopf_schema.json","name":"demo"}"#;
    std::fs::write(fixture.cwd.join("sparrowzz/dss/demo.rss"), rss).unwrap();
    std::fs::create_dir_all(fixture.cwd.join("psa/tests/bmopf")).unwrap();
    std::fs::write(fixture.cwd.join("psa/tests/bmopf/model.json"), model).unwrap();
    std::fs::create_dir_all(fixture.cwd.join("sparrowzz/dss/models")).unwrap();
    std::fs::write(
        fixture.cwd.join("sparrowzz/dss/models/network.dss"),
        "! This file is normally called by the \"Missing_Run_File.DSS\" file.\nbuscoords coordinates.csv\nnew circuit.demo\n",
    )
    .unwrap();
    std::fs::write(
        fixture.cwd.join("sparrowzz/dss/models/coordinates.csv"),
        "bus,x,y\n1,0,0\n",
    )
    .unwrap();
    fixture.contract.root_rss[0].sha256 = hash(rss);

    let snapshot =
        EngineResourceRegistry::load_with_contract(&fixture.cwd, fixture.contract, &HashSet::new())
            .unwrap();
    let example = snapshot
        .examples
        .iter()
        .find(|example| example.source_kind == "rss")
        .unwrap();
    assert!(example.importable, "{:?}", example.diagnostics);
    assert_eq!(example.files.len(), 4);
}

#[test]
fn rejects_missing_hash_mismatch_and_path_escape() {
    let missing = fixture(None);
    std::fs::remove_file(missing.cwd.join("sparrowzz/dss/docs/guide.md")).unwrap();
    assert!(
        EngineResourceRegistry::load_with_contract(&missing.cwd, missing.contract, &HashSet::new())
            .is_err()
    );

    let mismatch = fixture(None);
    std::fs::write(
        mismatch
            .cwd
            .join("sparrowzz/dss/docs/catalog/dataframes.json"),
        b"{}",
    )
    .unwrap();
    assert_eq!(
        EngineResourceRegistry::load_with_contract(
            &mismatch.cwd,
            mismatch.contract,
            &HashSet::new()
        )
        .unwrap_err()
        .code(),
        "engine_resource_hash_mismatch"
    );

    let mut escape = fixture(None);
    escape.contract.doc_paths = vec!["../outside.md".into()];
    assert_eq!(
        EngineResourceRegistry::load_with_contract(&escape.cwd, escape.contract, &HashSet::new())
            .unwrap_err()
            .code(),
        "engine_resource_path_escape"
    );

    let mut incompatible = fixture(None);
    incompatible.contract.catalog_schema_version = 3;
    assert_eq!(
        EngineResourceRegistry::load_with_contract(
            &incompatible.cwd,
            incompatible.contract,
            &HashSet::new()
        )
        .unwrap_err()
        .code(),
        "engine_contract_mismatch"
    );
}

#[test]
fn bundle_hash_dependency_and_required_feature_control_importability() {
    let required = fixture(Some("mumps"));
    let snapshot = EngineResourceRegistry::load_with_contract(
        &required.cwd,
        required.contract.clone(),
        &HashSet::new(),
    )
    .unwrap();
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
            .any(|item| item.contains("missing feature"))
    );

    let capabilities = HashSet::from(["mumps".to_string()]);
    let snapshot =
        EngineResourceRegistry::load_with_contract(&required.cwd, required.contract, &capabilities)
            .unwrap();
    assert!(
        snapshot
            .examples
            .iter()
            .find(|example| example.source_kind == "rssbundle")
            .unwrap()
            .importable
    );

    let broken = fixture(None);
    std::fs::write(
        broken
            .cwd
            .join("sparrowzz/dss/rssbundle/demo.rssbundle/main.rss"),
        b"changed",
    )
    .unwrap();
    let snapshot =
        EngineResourceRegistry::load_with_contract(&broken.cwd, broken.contract, &HashSet::new())
            .unwrap();
    assert!(
        !snapshot
            .examples
            .iter()
            .find(|example| example.source_kind == "rssbundle")
            .unwrap()
            .importable
    );
}

#[test]
fn current_eig_rs_tree_matches_the_embedded_release_contract_when_available() {
    let path = Path::new("../../eig-rs");
    if path
        .join("sparrowzz/dss/docs/catalog/dataframes.json")
        .is_file()
    {
        let snapshot = EngineResourceRegistry::load(path)
            .expect("configured eig-rs resources must match the pinned contract");
        assert_eq!(snapshot.examples.len(), 97);
    }
}
