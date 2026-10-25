use std::io::Write;

use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;
use zjudss::ingestion::{
    ArchiveExtractor, ArchiveLimits, BundleValidator, ExtractedTree, RssInspector,
};

fn zip_file(entries: &[(&str, &[u8], Option<u32>)]) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.zip");
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    for (name, bytes, mode) in entries {
        let mut options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if mode.is_some_and(|value| value & 0o170000 == 0o120000) {
            writer
                .add_symlink(*name, String::from_utf8_lossy(bytes), options)
                .unwrap();
            continue;
        } else if let Some(mode) = mode {
            options = options.unix_permissions(*mode);
        }
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    (root, path)
}

#[test]
fn archive_rejects_escape_symlink_duplicate_and_limits_without_partial_tree() {
    for (name, mode, expected) in [
        ("../escape.rss", None, "archive_path_escape"),
        ("link", Some(0o120777), "archive_symlink_forbidden"),
    ] {
        let (root, path) = zip_file(&[(name, b"x", mode)]);
        let out = root.path().join("out");
        let error = ArchiveExtractor::extract(&path, &out, ArchiveLimits::default()).unwrap_err();
        assert_eq!(error.code(), expected);
        assert!(!out.exists());
    }

    // ZipWriter 会主动清理绝对路径，因此在归档字节中把等长名称改成绝对路径，
    // 模拟来自不可信客户端的原始 ZIP。
    let (root, path) = zip_file(&[("xabsolute.rss", b"x", None)]);
    let mut bytes = std::fs::read(&path).unwrap();
    for offset in 0..=bytes.len() - b"xabsolute.rss".len() {
        if &bytes[offset..offset + b"xabsolute.rss".len()] == b"xabsolute.rss" {
            bytes[offset] = b'/';
        }
    }
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(
        ArchiveExtractor::extract(&path, &root.path().join("out"), ArchiveLimits::default())
            .unwrap_err()
            .code(),
        "archive_path_escape"
    );

    let (root, path) = zip_file(&[("same1.rss", b"a", None), ("same2.rss", b"b", None)]);
    let mut bytes = std::fs::read(&path).unwrap();
    for offset in 0..=bytes.len() - b"same1.rss".len() {
        if matches!(
            &bytes[offset..offset + b"same1.rss".len()],
            b"same1.rss" | b"same2.rss"
        ) {
            bytes[offset + 4] = b'0';
        }
    }
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(
        ArchiveExtractor::extract(&path, &root.path().join("out"), ArchiveLimits::default())
            .unwrap_err()
            .code(),
        "archive_duplicate_path"
    );

    let (root, path) = zip_file(&[("a", b"1", None), ("b", b"2", None)]);
    let limits = ArchiveLimits {
        max_entries: 1,
        ..ArchiveLimits::default()
    };
    assert_eq!(
        ArchiveExtractor::extract(&path, &root.path().join("out"), limits)
            .unwrap_err()
            .code(),
        "archive_too_many_entries"
    );

    let (root, path) = zip_file(&[("large", b"1234", None)]);
    let limits = ArchiveLimits {
        max_expanded_bytes: 3,
        ..ArchiveLimits::default()
    };
    assert_eq!(
        ArchiveExtractor::extract(&path, &root.path().join("out"), limits)
            .unwrap_err()
            .code(),
        "archive_expanded_too_large"
    );

    let repetitive = vec![0_u8; 32 * 1024];
    let (root, path) = zip_file(&[("bomb", &repetitive, None)]);
    let limits = ArchiveLimits {
        max_compression_ratio: 2,
        ..ArchiveLimits::default()
    };
    assert_eq!(
        ArchiveExtractor::extract(&path, &root.path().join("out"), limits)
            .unwrap_err()
            .code(),
        "archive_compression_ratio_exceeded"
    );
}

#[test]
fn zip_and_directory_bundle_have_the_same_canonical_manifest() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("demo.rssbundle");
    std::fs::create_dir_all(directory.join("dss")).unwrap();
    let entry = b"x = import_dss(\"dss/model.dss\");";
    let model = b"New Circuit.demo";
    std::fs::write(directory.join("main.rss"), entry).unwrap();
    std::fs::write(directory.join("dss/model.dss"), model).unwrap();
    let hash = |bytes: &[u8]| hex::encode(Sha256::digest(bytes));
    let manifest = serde_json::to_vec(&serde_json::json!({
        "entry":"main.rss",
        "files":[
            {"path":"dss/model.dss","sha256":hash(model)},
            {"path":"main.rss","sha256":hash(entry)}
        ],
        "required_features":[],
        "study":{"kind":"power_flow","solver":"radial"}
    }))
    .unwrap();
    std::fs::write(directory.join("manifest.json"), &manifest).unwrap();
    let direct = ExtractedTree::from_files(
        root.path().to_path_buf(),
        [
            ("demo.rssbundle/main.rss".into(), directory.join("main.rss")),
            (
                "demo.rssbundle/dss/model.dss".into(),
                directory.join("dss/model.dss"),
            ),
            (
                "demo.rssbundle/manifest.json".into(),
                directory.join("manifest.json"),
            ),
        ],
    )
    .unwrap();
    let direct = BundleValidator::validate(&direct).unwrap();
    assert!(direct.diagnostics.is_empty());

    let (zip_root, zip_path) = zip_file(&[
        ("demo.rssbundle/main.rss", entry, None),
        ("demo.rssbundle/dss/model.dss", model, None),
        ("demo.rssbundle/manifest.json", &manifest, None),
    ]);
    let extracted = ArchiveExtractor::extract(
        &zip_path,
        &zip_root.path().join("out"),
        ArchiveLimits::default(),
    )
    .unwrap();
    let zipped = BundleValidator::validate(&extracted).unwrap();
    assert_eq!(direct.canonical_manifest, zipped.canonical_manifest);
}

#[test]
fn rss_missing_dependency_is_a_collecting_diagnostic_and_manifest_hash_errors_are_structured() {
    let root = tempfile::tempdir().unwrap();
    let rss = root.path().join("main.rss");
    std::fs::write(&rss, b"x = import_dss(\"missing/model.dss\");").unwrap();
    let tree =
        ExtractedTree::from_files(root.path().to_path_buf(), [("main.rss".into(), rss)]).unwrap();
    let report = RssInspector::inspect("main.rss", &tree).unwrap();
    assert_eq!(report.missing, ["missing/model.dss"]);

    let bundle = root.path().join("broken.rssbundle");
    std::fs::create_dir_all(&bundle).unwrap();
    let entry = bundle.join("main.rss");
    std::fs::write(&entry, b"x=1").unwrap();
    let manifest_path = bundle.join("manifest.json");
    std::fs::write(
        &manifest_path,
        br#"{"entry":"main.rss","files":[{"path":"main.rss","sha256":"wrong"}]}"#,
    )
    .unwrap();
    let tree = ExtractedTree::from_files(
        root.path().to_path_buf(),
        [
            ("broken.rssbundle/main.rss".into(), entry),
            ("broken.rssbundle/manifest.json".into(), manifest_path),
        ],
    )
    .unwrap();
    let validated = BundleValidator::validate(&tree).unwrap();
    assert!(
        validated
            .diagnostics
            .iter()
            .any(|item| item.contains("hash mismatch"))
    );
}
