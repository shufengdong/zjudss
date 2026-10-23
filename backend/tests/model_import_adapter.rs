use std::time::Duration;

use backend::adapters::{
    ImportInspectSpec, ImportProfile, ImportSpec, STATIC_MODEL_TABLES, SimEngine, StubEngine,
};

#[tokio::test]
async fn include_graph_returns_stable_virtual_root_candidates() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("master.dss"), "Redirect parts/line.dss\n").unwrap();
    std::fs::create_dir(root.path().join("parts")).unwrap();
    std::fs::write(root.path().join("parts/line.dss"), "New Line.L1\n").unwrap();
    std::fs::write(root.path().join("other.dss"), "New Circuit.Other\n").unwrap();

    let engine = StubEngine;
    let first = engine
        .inspect_model_import(ImportInspectSpec {
            input_tree: root.path().to_path_buf(),
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    let second = engine
        .inspect_model_import(ImportInspectSpec {
            input_tree: root.path().to_path_buf(),
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    assert_eq!(first.entry_candidates, second.entry_candidates);
    assert_eq!(first.entry_candidates.len(), 2);
    assert_eq!(
        first
            .entry_candidates
            .iter()
            .map(|candidate| candidate.display_path.as_str())
            .collect::<Vec<_>>(),
        ["master.dss", "other.dss"]
    );
    assert!(first.entry_candidates.iter().all(|candidate| {
        !candidate.id.contains('/') && !candidate.id.contains(root.path().to_str().unwrap())
    }));
}

#[tokio::test]
async fn static_import_emits_all_canonical_snappy_tables_including_zero_rows() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("master.dss"), "New Circuit.Demo\n").unwrap();
    let output = root.path().join("out");
    let engine = StubEngine;
    let inspection = engine
        .inspect_model_import(ImportInspectSpec {
            input_tree: root.path().to_path_buf(),
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    let imported = engine
        .import_model(ImportSpec {
            input_tree: root.path().to_path_buf(),
            entry_candidate_id: inspection.entry_candidates[0].id.clone(),
            result_dir: output,
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap();
    assert_eq!(imported.profile, ImportProfile::StaticDss);
    assert_eq!(imported.dataframes.len(), 21);
    assert_eq!(
        imported
            .dataframes
            .iter()
            .map(|dataset| dataset.name.as_str())
            .collect::<Vec<_>>(),
        STATIC_MODEL_TABLES
    );
    assert!(
        imported
            .dataframes
            .iter()
            .all(|dataset| dataset.metadata.snappy)
    );
    assert!(
        imported
            .dataframes
            .iter()
            .any(|dataset| dataset.metadata.row_count == 0)
    );
    assert!(imported.dataframes.iter().all(|dataset| {
        dataset.path.starts_with(root.path())
            && dataset.path.extension() == Some("parquet".as_ref())
    }));
}

#[tokio::test]
async fn unsupported_execution_lifecycles_fail_before_outputs() {
    for source in [
        "Solve\nSolve\n",
        "Set Mode=Harmonic\nSolve\n",
        "AutoAdd\n",
        "RelCalc\n",
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("master.dss"), source).unwrap();
        let engine = StubEngine;
        let inspection = engine
            .inspect_model_import(ImportInspectSpec {
                input_tree: root.path().to_path_buf(),
                timeout: Duration::from_secs(1),
            })
            .await
            .unwrap();
        let error = engine
            .import_model(ImportSpec {
                input_tree: root.path().to_path_buf(),
                entry_candidate_id: inspection.entry_candidates[0].id.clone(),
                result_dir: root.path().join("out"),
                timeout: Duration::from_secs(1),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "model_import_unsupported");
        assert!(!root.path().join("out").exists());
    }
}

#[test]
fn import_specs_reject_paths_outside_the_source_tree() {
    let root = tempfile::tempdir().unwrap();
    let spec = ImportSpec {
        input_tree: root.path().to_path_buf(),
        entry_candidate_id: "../master.dss".into(),
        result_dir: root.path().join("output"),
        timeout: Duration::from_secs(1),
    };
    assert_eq!(spec.validate().unwrap_err().code, "entry_candidate_invalid");
}
