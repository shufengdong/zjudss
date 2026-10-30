use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zjudss::db;
use zjudss::db::repositories::EngineResourceSnapshotRepo;
use zjudss::engine_resources::{
    EngineResourceRegistry, ResourceLimits, SUPPORTED_CASE_PROJECTIONS_SCHEMA_VERSION,
    SUPPORTED_DATAFRAMES_SCHEMA_VERSION, SUPPORTED_DTYPE_GRAMMAR_VERSION,
    SUPPORTED_FUNCTIONS_SCHEMA_VERSION, SUPPORTED_STUDY_FACADES_SCHEMA_VERSION,
};

const DATAFRAMES: &[u8] = include_bytes!("fixtures/engine_resources/catalog/dataframes.json");
const FUNCTIONS: &[u8] = include_bytes!("fixtures/engine_resources/catalog/functions.json");
const STUDY_FACADES: &[u8] = include_bytes!("fixtures/engine_resources/catalog/study_facades.json");
const CASE_PROJECTIONS: &[u8] =
    include_bytes!("fixtures/engine_resources/catalog/case_projections.json");
const MANAGED_RSS: &[u8] = b"[bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para] = project_dss_static_model(\"dss/model.dss\");\n[voltage] = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);\n";

fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct Fixture {
    _root: tempfile::TempDir,
    sparrowzz_dir: PathBuf,
}
impl Fixture {
    fn dss(&self) -> PathBuf {
        self.sparrowzz_dir.join("dss")
    }
    fn bundle_root(&self) -> PathBuf {
        self.dss().join("rssbundle/Grid/123Bus/demo.rssbundle")
    }
    fn load(&self) -> zjudss::engine_resources::EngineResourceSnapshot {
        EngineResourceRegistry::load(&self.sparrowzz_dir, &HashSet::new()).unwrap()
    }
}

fn valid_manifest(main: &[u8], model: &[u8]) -> Value {
    json!({
        "schema_version": 2, "artifact": "mems.dss-rss-bundle",
        "generator": "engine-resource-test/1.0", "psa_contract_version": "0.1.0",
        "entry": "main.rss",
        "study": {"kind":"static_power_flow","compatibility":"opendss-native","solver":"newton","results":["voltage"]},
        "source": {"entry":"dss/model.dss","root":"dss","tree_sha256":hash(model)},
        "projection": {
            "source_function": "ds_newton_raphson_pf",
            "profile": "dss_static_newton_v1",
            "projector": "project_dss_static_model",
            "target_function": "ds_newton_raphson_pf",
            "managed_entry": "managed.rss",
            "status": "enabled",
            "stable_diagnostic": "PSA.DSS.PROJECTION.INVALID",
            "results": [{"source":"voltage","target":"bus_phase","binding":"result_voltage"}]
        },
        "files": [
            {"path":"dss/model.dss","role":"dss-entry","sha256":hash(model)},
            {"path":"main.rss","role":"main-rss","sha256":hash(main)},
            {"path":"managed.rss","role":"managed-rss","sha256":hash(MANAGED_RSS)}
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
    std::fs::write(bundle.join("managed.rss"), MANAGED_RSS).unwrap();
    std::fs::write(
        bundle.join("manifest.json"),
        serde_json::to_vec_pretty(manifest).unwrap(),
    )
    .unwrap();
}

fn fixture() -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let sparrowzz_dir = root.path().join("sparrowzz");
    let dss = sparrowzz_dir.join("dss");
    std::fs::create_dir_all(dss.join("docs/catalog")).unwrap();
    std::fs::create_dir_all(dss.join("docs/nested")).unwrap();
    std::fs::write(dss.join("docs/catalog/dataframes.json"), DATAFRAMES).unwrap();
    std::fs::write(dss.join("docs/catalog/functions.json"), FUNCTIONS).unwrap();
    std::fs::write(dss.join("docs/catalog/study_facades.json"), STUDY_FACADES).unwrap();
    std::fs::write(
        dss.join("docs/catalog/case_projections.json"),
        CASE_PROJECTIONS,
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
    Fixture {
        _root: root,
        sparrowzz_dir,
    }
}

fn rewrite_catalog(fixture: &Fixture, file_name: &str, change: impl FnOnce(&mut Value)) {
    let path = fixture.dss().join("docs/catalog").join(file_name);
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    change(&mut value);
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn accepts_independent_supported_catalog_versions() {
    let snapshot = fixture().load();
    assert_eq!(snapshot.catalog_schema_versions.functions, 2);
    assert_eq!(snapshot.catalog_schema_versions.study_facades, 1);
    assert_eq!(snapshot.catalog_schema_versions.case_projections, 2);
    assert_eq!(snapshot.catalog_schema_versions.dataframes, 4);
    assert_eq!(snapshot.catalog_schema_versions.dtype_grammar, 1);
}

#[test]
fn accepts_paired_presence_as_an_executable_dataframe_rule() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "dataframes.json", |root| {
        root["frames"]["bus"]["rules"] = json!([
            {
                "type": "paired_presence",
                "fields": ["id", "terminals"]
            },
            {
                "type": "equal_length",
                "fields": ["id", "terminals"],
                "minimumLength": 1
            }
        ]);
    });
    let snapshot = fixture.load();
    assert_eq!(
        snapshot.facades.enabled_count(),
        snapshot.facades.catalog_entry_count()
    );
    assert!(snapshot.facades.diagnostics().is_empty());
}

#[test]
fn rejects_each_unsupported_catalog_or_dtype_grammar_version() {
    let cases = [
        ("functions.json", vec!["schema_version"]),
        ("study_facades.json", vec!["schema_version"]),
        ("case_projections.json", vec!["schema_version"]),
        ("dataframes.json", vec!["schema_version"]),
        ("dataframes.json", vec!["dtype_grammar", "version"]),
    ];
    for (file, path) in cases {
        let fixture = fixture();
        rewrite_catalog(&fixture, file, |root| {
            let mut current = root;
            for part in &path[..path.len() - 1] {
                current = &mut current[*part];
            }
            current[path[path.len() - 1]] = json!(999);
        });
        assert_eq!(
            EngineResourceRegistry::load(&fixture.sparrowzz_dir, &HashSet::new())
                .unwrap_err()
                .code(),
            "engine_contract_mismatch"
        );
    }
}

#[test]
fn publishes_enabled_case_projection_with_ordered_outputs() {
    let snapshot = fixture().load();
    assert_eq!(SUPPORTED_CASE_PROJECTIONS_SCHEMA_VERSION, 2);
    assert_eq!(snapshot.case_projections.catalog_entry_count(), 2);
    assert_eq!(
        snapshot.case_projections.enabled_count(),
        1,
        "{:?}",
        snapshot.case_projections.diagnostics()
    );
    assert_eq!(snapshot.case_projections.diagnostics().len(), 1);
    let profile = snapshot
        .case_projections
        .get_enabled("dss_static_newton_v1")
        .unwrap();
    assert_eq!(profile.projector, "project_dss_static_model");
    assert_eq!(profile.target_function, "ds_newton_raphson_pf");
    assert_eq!(profile.projector_outputs.len(), 22);
    assert_eq!(profile.projector_outputs[0].position, 1);
    assert_eq!(profile.projector_outputs[20].alias, "para");
}

#[test]
fn disables_case_projection_with_a_result_selector_that_does_not_match_the_target_facade() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "case_projections.json", |root| {
        root["profiles"][0]["resultSelector"]["field"] = json!("wrong_result_field");
    });
    let snapshot = fixture.load();
    assert!(
        snapshot
            .case_projections
            .get_enabled("dss_static_newton_v1")
            .is_none()
    );
    assert_eq!(
        snapshot
            .case_projections
            .diagnostic("dss_static_newton_v1")
            .unwrap()
            .code,
        "case_projection_invalid"
    );
}

#[test]
fn accepts_contiguous_zero_based_as_an_executable_dataframe_rule() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "dataframes.json", |root| {
        root["frames"]["bus"]["contract"]["columns"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "name":"snapshot_index", "dtype":"u32", "required":true,
                "nullable":false, "unit":"not_applicable"
            }));
        root["frames"]["bus"]["contract"]["rules"] = json!([
            {"type":"contiguous_zero_based", "field":"snapshot_index"}
        ]);
    });
    let snapshot = fixture.load();
    assert!(snapshot.facades.diagnostics().is_empty());
}

#[test]
fn snapshot_hash_includes_study_facades_and_all_version_numbers() {
    let fixture = fixture();
    let first = fixture.load();
    rewrite_catalog(&fixture, "study_facades.json", |root| {
        root["source_of_truth"] = json!(["changed presentation source"]);
    });
    let second = fixture.load();
    assert_ne!(first.id, second.id);
    assert_ne!(first.study_facades_sha256, second.study_facades_sha256);
    assert_eq!(
        first.catalog_schema_versions,
        second.catalog_schema_versions
    );
}

#[test]
fn snapshot_hash_includes_case_projection_catalog() {
    let fixture = fixture();
    let first = fixture.load();
    rewrite_catalog(&fixture, "case_projections.json", |root| {
        root["source_of_truth"] = json!(["changed projection source"]);
    });
    let second = fixture.load();
    assert_ne!(first.id, second.id);
    assert_ne!(
        first.case_projections_sha256,
        second.case_projections_sha256
    );
}

#[test]
fn publishes_every_valid_dataframe_facade() {
    let snapshot = fixture().load();
    assert_eq!(snapshot.facades.catalog_entry_count(), 15);
    assert_eq!(
        snapshot.facades.enabled_count(),
        15,
        "{:?}",
        snapshot.facades.diagnostics()
    );
    assert!(snapshot.facades.diagnostics().is_empty());
    assert!(
        snapshot
            .facades
            .get_enabled("static_power_flow_newton")
            .is_some()
    );
}

#[test]
fn missing_study_facades_publishes_one_global_diagnostic() {
    let fixture = fixture();
    std::fs::remove_file(fixture.dss().join("docs/catalog/study_facades.json")).unwrap();

    let snapshot = fixture.load();

    assert_eq!(snapshot.facades.catalog_entry_count(), 0);
    assert_eq!(snapshot.facades.enabled_count(), 0);
    assert_eq!(snapshot.facades.diagnostics().len(), 1);
    assert_eq!(
        snapshot.facades.diagnostics()[0].code,
        "study_facades_missing"
    );
}

#[tokio::test]
async fn publishing_snapshot_metadata_atomically_marks_one_current_snapshot() {
    let fixture = fixture();
    let first = fixture.load();
    rewrite_catalog(&fixture, "study_facades.json", |root| {
        root["source_of_truth"] = json!(["second snapshot"]);
    });
    let second = fixture.load();
    let database = tempfile::NamedTempFile::new().unwrap();
    let pool = db::establish_pool(&format!("sqlite://{}", database.path().display())).unwrap();
    {
        let mut connection = pool.get().unwrap();
        db::run_migrations(&mut connection).unwrap();
    }
    let repo = EngineResourceSnapshotRepo::new(pool);

    repo.publish_current(&first, 1).await.unwrap();
    repo.publish_current(&second, 2).await.unwrap();

    let current = repo.current().await.unwrap().unwrap();
    assert_eq!(current.id, second.id);
    assert!(current.is_current);
    assert_eq!(current.functions_schema_version, 2);
    assert_eq!(current.study_facades_schema_version, 1);
    assert_eq!(current.case_projections_schema_version, 2);
    assert_eq!(current.dataframes_schema_version, 4);
    assert_eq!(current.dtype_grammar_version, 1);
    assert_eq!(repo.count().await.unwrap(), 2);
}

#[test]
fn excludes_path_url_and_import_facades() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "study_facades.json", |root| {
        let mut facade = root["facades"][0].clone();
        facade["id"] = json!("unsafe_import");
        facade["function"] = json!("import_dss_url");
        facade["inputs"] = json!([{
            "id":"source", "alias":"source", "inputKind":"url",
            "schemaRef":"frames.bus", "required":true, "position":1
        }]);
        root["facades"].as_array_mut().unwrap().push(facade);
    });
    let snapshot = fixture.load();
    assert_eq!(snapshot.facades.catalog_entry_count(), 16);
    assert_eq!(snapshot.facades.enabled_count(), 15);
    assert!(snapshot.facades.get_enabled("unsafe_import").is_none());
    assert_eq!(
        snapshot.facades.diagnostic("unsafe_import").unwrap().code,
        "facade_unsupported_input_kind"
    );
}

#[test]
fn invalid_facade_is_disabled_without_disabling_neighbors() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "study_facades.json", |root| {
        root["facades"][0]["inputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("inputKind");
    });
    let snapshot = fixture.load();
    assert_eq!(snapshot.facades.enabled_count(), 14);
    assert!(
        snapshot
            .facades
            .get_enabled("static_power_flow_radial")
            .is_none()
    );
    assert!(
        snapshot
            .facades
            .get_enabled("static_power_flow_newton")
            .is_some()
    );
    assert_eq!(snapshot.facades.diagnostics().len(), 1);
    let explicit = snapshot.facades.search("", Some("ds_radial_pf"));
    assert!(explicit.enabled.is_empty());
    assert_eq!(explicit.disabled.len(), 1);
}

#[test]
fn incomplete_transitive_schema_disables_only_affected_facades() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "dataframes.json", |root| {
        root["result_frames"]["bus_voltage"]["contract"]["columns"][0]
            .as_object_mut()
            .unwrap()
            .remove("dtype");
    });
    let snapshot = fixture.load();
    assert!(
        snapshot
            .facades
            .get_enabled("static_power_flow_radial")
            .is_none()
    );
    assert!(
        snapshot.facades.get_enabled("fault_study").is_some(),
        "{:?}",
        snapshot.facades.diagnostics()
    );
    assert!(snapshot.facades.enabled_count() > 0);
}

#[test]
fn prose_constraints_do_not_satisfy_struct_card_contract() {
    let fixture = fixture();
    rewrite_catalog(&fixture, "dataframes.json", |root| {
        root["frames"]["para"]["constraints"] =
            json!(["the omitted machine-readable card exists in prose"]);
        root["frames"]["para"]["contract"]["columns"][0]
            .as_object_mut()
            .unwrap()
            .remove("card");
    });
    let snapshot = fixture.load();
    let diagnostic = snapshot
        .facades
        .diagnostic("static_power_flow_radial")
        .unwrap();
    assert_eq!(diagnostic.code, "facade_schema_invalid");
    assert!(diagnostic.message.contains("card"));
}

#[test]
fn base_schema_resolves_once_without_cycles() {
    let snapshot = fixture().load();
    let qsts = snapshot
        .schemas
        .resolve("qsts_outputs.bus_voltage")
        .unwrap();
    let names = qsts
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names.iter().filter(|name| **name == "bus").count(), 1);
    assert!(names.contains(&"time"));

    let cyclic = fixture();
    rewrite_catalog(&cyclic, "dataframes.json", |root| {
        root["qsts_outputs"]["bus_voltage"]["base_schema"] = json!("qsts_outputs.bus_voltage");
    });
    let cyclic = cyclic.load();
    assert!(cyclic.facades.get_enabled("qsts_radial").is_none());
    assert!(
        cyclic
            .facades
            .get_enabled("static_power_flow_radial")
            .is_some()
    );
}

#[test]
fn facade_hash_changes_with_transitive_executable_schema() {
    let fixture = fixture();
    let first = fixture.load();
    let first_hash = first
        .facades
        .get_enabled("static_power_flow_radial")
        .unwrap()
        .contract_hash
        .clone();
    rewrite_catalog(&fixture, "dataframes.json", |root| {
        root["result_frames"]["bus_voltage"]["contract"]["columns"][1]["unit"] = json!("kV");
    });
    let second = fixture.load();
    assert_ne!(
        first_hash,
        second
            .facades
            .get_enabled("static_power_flow_radial")
            .unwrap()
            .contract_hash
    );
}

#[test]
fn facade_hash_ignores_presentation_only_text() {
    let fixture = fixture();
    let first = fixture.load();
    let first_hash = first
        .facades
        .get_enabled("static_power_flow_radial")
        .unwrap()
        .contract_hash
        .clone();
    rewrite_catalog(&fixture, "study_facades.json", |root| {
        root["facades"][0]["title"] = json!("新的展示标题");
        root["facades"][0]["description"] = json!("新的展示说明");
        root["facades"][0]["differences"] = json!(["新的展示差异"]);
    });
    let second = fixture.load();
    assert_eq!(
        first_hash,
        second
            .facades
            .get_enabled("static_power_flow_radial")
            .unwrap()
            .contract_hash
    );
}

#[test]
fn discovers_current_tree_deterministically_and_snapshot_changes_with_loaded_bytes() {
    assert_eq!(SUPPORTED_FUNCTIONS_SCHEMA_VERSION, 2);
    assert_eq!(SUPPORTED_STUDY_FACADES_SCHEMA_VERSION, 1);
    assert_eq!(SUPPORTED_DATAFRAMES_SCHEMA_VERSION, 4);
    assert_eq!(SUPPORTED_DTYPE_GRAMMAR_VERSION, 1);
    let fixture = fixture();
    let first = fixture.load();
    assert_eq!(first.id, fixture.load().id);
    assert_eq!(first.examples.len(), 1);
    let example = &first.examples[0];
    assert_eq!(example.category_path, ["Grid", "123Bus"]);
    assert_eq!(example.bundle_path, "Grid/123Bus/demo.rssbundle");
    assert_eq!(example.entry_path.as_deref(), Some("main.rss"));
    let manifest = example.manifest.as_ref().unwrap();
    assert_eq!(manifest.schema_version, 2);
    assert_eq!(manifest.artifact, "mems.dss-rss-bundle");
    assert_eq!(manifest.generator, "engine-resource-test/1.0");
    assert_eq!(manifest.psa_contract_version, "0.1.0");
    assert_eq!(manifest.study.kind, "static_power_flow");
    assert_eq!(manifest.study.compatibility, "opendss-native");
    assert_eq!(manifest.study.solver.as_deref(), Some("newton"));
    assert_eq!(manifest.study.results, ["voltage"]);
    assert_eq!(manifest.projection.profile, "dss_static_newton_v1");
    assert_eq!(manifest.projection.managed_entry, "managed.rss");
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
    schema["schema_version"] = json!(3);
    write_case(&root.join("schema.rssbundle"), &schema);
    let mut artifact = valid_manifest(main, model);
    artifact["artifact"] = json!("other.bundle");
    write_case(&root.join("artifact.rssbundle"), &artifact);
    let mut projection = valid_manifest(main, model);
    projection["projection"]["projector"] = json!("wrong_projector");
    write_case(&root.join("projection.rssbundle"), &projection);
    let mut disabled = valid_manifest(main, model);
    disabled["projection"]["profile"] = json!("disabled_projection_v1");
    write_case(&root.join("disabled.rssbundle"), &disabled);
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
    let projection = snapshot
        .examples
        .iter()
        .find(|item| item.title == "projection")
        .unwrap();
    assert!(!projection.importable);
    assert_eq!(
        projection.diagnostics,
        ["PSA.DSS.PROJECTION.INVALID: projection_contract_mismatch"]
    );
    assert!(projection.manifest.is_some());
    let disabled = snapshot
        .examples
        .iter()
        .find(|item| item.title == "disabled")
        .unwrap();
    assert!(!disabled.importable);
    assert_eq!(disabled.diagnostics, ["PSA.DSS.PROJECTION.DISABLED"]);
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
    assert!(!serialized.contains(fixture.sparrowzz_dir.to_string_lossy().as_ref()));
}

#[test]
fn bundle_manifest_without_a_dss_source_is_not_importable() {
    let fixture = fixture();
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let model = b"new circuit.demo basekv=12.47\n";
    let mut manifest = valid_manifest(main, model);
    manifest["files"] = json!([
        {"path":"main.rss","role":"main-rss","sha256":hash(main)},
        {"path":"managed.rss","role":"managed-rss","sha256":hash(MANAGED_RSS)}
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
    assert!(!path.join("source/dss/demo.rss").exists());
    let bundle = path.join("source/dss/rssbundle/Grid/123Bus/demo.rssbundle");
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
        path.join("source/dss/rssbundle/Grid/123Bus/demo.rssbundle/dss/model.dss"),
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
    rewrite_catalog(&unsupported, "dataframes.json", |root| {
        root["schema_version"] = json!(999);
    });
    assert_eq!(
        EngineResourceRegistry::load(&unsupported.sparrowzz_dir, &HashSet::new())
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
        EngineResourceRegistry::load(&malformed.sparrowzz_dir, &HashSet::new())
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
    type LimitCase = Box<dyn Fn(&Fixture) -> ResourceLimits>;

    let cases: Vec<LimitCase> = vec![
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
                &fixture.sparrowzz_dir,
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
    let outside = fixture._root.path().join("outside.md");
    std::fs::write(&outside, "outside").unwrap();
    symlink(&outside, fixture.dss().join("docs/linked.md")).unwrap();
    assert_eq!(
        EngineResourceRegistry::load(&fixture.sparrowzz_dir, &HashSet::new())
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
    let path = PathBuf::from("../../eig-rs/sparrowzz");
    if path.join("dss/docs/catalog/dataframes.json").is_file() {
        let snapshot = EngineResourceRegistry::load(&path, &HashSet::new())
            .expect("current configured eig-rs resources must be dynamically valid");
        assert_eq!(snapshot.catalog_schema_versions.dataframes, 4);
        assert_eq!(snapshot.catalog_schema_versions.case_projections, 2);
        assert_eq!(snapshot.facades.enabled_count(), 18);
        assert_eq!(
            snapshot.facades.enabled_count(),
            snapshot.facades.catalog_entry_count()
        );
        assert!(snapshot.facades.diagnostics().is_empty());
        assert!(snapshot.case_projections.enabled_count() > 0);
        assert_eq!(
            snapshot.case_projections.enabled_count()
                + snapshot.case_projections.diagnostics().len(),
            snapshot.case_projections.catalog_entry_count()
        );
        assert!(
            snapshot
                .examples
                .iter()
                .all(|item| item.source_kind == "rssbundle")
        );
        assert!(!snapshot.examples.is_empty());
        assert!(
            snapshot.examples.iter().all(|item| item.importable),
            "current eig-rs cases must all have enabled projection contracts: {:?}",
            snapshot
                .examples
                .iter()
                .filter(|item| !item.importable)
                .map(|item| (&item.bundle_path, &item.diagnostics))
                .take(10)
                .collect::<Vec<_>>()
        );
    }
}
