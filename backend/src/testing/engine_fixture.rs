use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::catalog::SchemaCatalog;
use crate::engine_resources::{EngineResourceRegistry, EngineResourceSnapshot};

use super::E2eCatalogMode;

pub fn engine_fixture_root(data_dir: &Path) -> PathBuf {
    data_dir.parent().unwrap_or(data_dir).join("engine-fixture")
}

pub fn prepare_engine_fixture(
    data_dir: &Path,
    catalog_mode: E2eCatalogMode,
) -> (Arc<EngineResourceSnapshot>, PathBuf) {
    let engine_root = engine_fixture_root(data_dir);
    let dss_root = engine_root.join("dss");
    std::fs::create_dir_all(dss_root.join("docs/catalog"))
        .expect("create test engine resource fixture");

    write_catalog(&dss_root);
    write_dependency_fixture(&engine_root, &dss_root);
    write_example_fixture(&dss_root);

    let engine_root = engine_root
        .canonicalize()
        .expect("canonicalize e2e engine root");
    let loaded = EngineResourceRegistry::load(&engine_root, &HashSet::new())
        .expect("load complete e2e engine resources");
    let snapshot = match catalog_mode {
        E2eCatalogMode::Full => {
            EngineResourceRegistry::publish(loaded, data_dir)
                .expect("publish isolated engine snapshot")
                .0
        }
        E2eCatalogMode::Minimal => {
            EngineResourceSnapshot::minimal("e2e-snapshot", "e2e-contract", engine_root.join("dss"))
        }
    };
    SchemaCatalog::from_value(&snapshot.contract_version, &snapshot.catalog_json)
        .expect("e2e catalog valid");
    (Arc::new(snapshot), engine_root)
}

fn write_catalog(dss_root: &Path) {
    let catalog_dir = dss_root.join("docs/catalog");
    let fixtures = [
        (
            "dataframes.json",
            include_bytes!("../../tests/fixtures/engine_resources/catalog/dataframes.json")
                .as_slice(),
        ),
        (
            "functions.json",
            include_bytes!("../../tests/fixtures/engine_resources/catalog/functions.json")
                .as_slice(),
        ),
        (
            "study_facades.json",
            include_bytes!("../../tests/fixtures/engine_resources/catalog/study_facades.json")
                .as_slice(),
        ),
        (
            "case_projections.json",
            include_bytes!("../../tests/fixtures/engine_resources/catalog/case_projections.json")
                .as_slice(),
        ),
    ];
    for (name, bytes) in fixtures {
        std::fs::write(catalog_dir.join(name), bytes).expect("write e2e catalog fixture");
    }
    std::fs::write(
        dss_root.join("docs/guide.md"),
        b"# Test engine guide\nfixture content\n",
    )
    .expect("write e2e guide");
}

fn write_dependency_fixture(engine_root: &Path, dss_root: &Path) {
    std::fs::create_dir_all(dss_root.join("models/demo")).expect("create dss fixture");
    std::fs::create_dir_all(engine_root.join("psa/tests/bmopf")).expect("create psa fixture");
    std::fs::create_dir_all(engine_root.join("rspower/lib")).expect("create rspower fixture");
    std::fs::write(
        dss_root.join("demo.rss"),
        b"a=import_dss(\"dss/models/demo/master.dss\");\nb=import_bmopf(\"psa/tests/bmopf/demo.json\");\nc=read(\"rspower/lib/helper.csv\");\n",
    )
    .expect("write rss dependency fixture");
    std::fs::write(
        dss_root.join("models/demo/master.dss"),
        b"Redirect child.dss\n",
    )
    .expect("write dss master fixture");
    std::fs::write(
        dss_root.join("models/demo/child.dss"),
        b"New Circuit.demo\n",
    )
    .expect("write dss child fixture");
    std::fs::write(engine_root.join("psa/tests/bmopf/demo.json"), b"{}\n")
        .expect("write psa fixture");
    std::fs::write(engine_root.join("rspower/lib/helper.csv"), b"id\n1\n")
        .expect("write rspower fixture");
}

fn write_example_fixture(dss_root: &Path) {
    let bundle = dss_root.join("rssbundle/E2E/Demo/E2E demo.rssbundle");
    std::fs::create_dir_all(bundle.join("dss")).expect("create e2e bundle fixture");
    let main = b"model = import_dss(\"dss/model.dss\");\n";
    let managed = b"[bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para] = project_dss_static_model(\"dss/model.dss\");\n[voltage] = ds_newton_raphson_pf(bus, line, linecode, load, transformer, voltage_source, motor, transformer_winding, transformer_impedance, current_source, generator, ibr, switch, shunt, capacitor, reactor, spectrum, control_profile, xy_curve, storage, para);\n";
    let source = b"new circuit.e2e basekv=10\n";
    std::fs::write(bundle.join("main.rss"), main).expect("write e2e main");
    std::fs::write(bundle.join("managed.rss"), managed).expect("write e2e managed");
    std::fs::write(bundle.join("dss/model.dss"), source).expect("write e2e source");

    let manifest = serde_json::json!({
        "schema_version": 2,
        "artifact": "mems.dss-rss-bundle",
        "generator": "zjudss-test/1",
        "psa_contract_version": "0.1.0",
        "entry": "main.rss",
        "study": {
            "kind": "static_power_flow",
            "compatibility": "opendss-native",
            "solver": "newton",
            "results": ["voltage"]
        },
        "source": {
            "entry": "dss/model.dss",
            "root": "dss",
            "tree_sha256": hex::encode(Sha256::digest(source))
        },
        "projection": {
            "source_function": "ds_newton_raphson_pf",
            "profile": "dss_static_newton_v1",
            "projector": "project_dss_static_model",
            "target_function": "ds_newton_raphson_pf",
            "managed_entry": "managed.rss",
            "status": "enabled",
            "stable_diagnostic": "PSA.DSS.PROJECTION.INVALID",
            "results": [
                {"source": "voltage", "target": "bus_phase", "binding": "result_voltage"}
            ]
        },
        "files": [
            {"path": "dss/model.dss", "role": "dss-entry", "sha256": hex::encode(Sha256::digest(source))},
            {"path": "main.rss", "role": "main-rss", "sha256": hex::encode(Sha256::digest(main))},
            {"path": "managed.rss", "role": "managed-rss", "sha256": hex::encode(Sha256::digest(managed))}
        ],
        "inputs": []
    });
    std::fs::write(
        bundle.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize e2e manifest"),
    )
    .expect("write e2e manifest");

    let broken = dss_root.join("rssbundle/E2E/Demo/Broken.rssbundle");
    std::fs::create_dir_all(&broken).expect("create invalid e2e bundle fixture");
    std::fs::write(broken.join("manifest.json"), b"{").expect("write invalid e2e manifest");
}
