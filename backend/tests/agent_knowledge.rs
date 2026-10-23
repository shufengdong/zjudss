use std::collections::BTreeMap;

use backend::agent::knowledge::AgentKnowledge;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn fixture(
    index: Value,
    files: BTreeMap<String, String>,
) -> (String, String, String, BTreeMap<String, String>) {
    let index = index.to_string();
    let system = "Stable system rules".to_string();
    let manifest = json!({
        "agentBundleVersion":"test", "engineContractVersion":"contract-1",
        "eigRsCommit":"a", "sparrowzzCommit":"b", "sourceDirty":false,
        "catalogSchemaVersion":2, "compatibleCatalogSchemaVersions":[2],
        "catalogs":{"dataframes":"x","functions":"y"}, "docsTreeSha256":"z",
        "examplesSourceSha256":"e", "systemSha256":hash(&system),
        "skillIndexSha256":hash(&index), "allowedTools":["known_tool"]
    })
    .to_string();
    (manifest, index, system, files)
}

fn skill(id: &str, path: &str, tool: &str, content: &str) -> Value {
    json!({
        "id":id,"version":"1.0.0","description":"test","triggers":[],
        "requiredTools":[tool],"engineContractVersion":"contract-1","sources":[],
        "path":path,"sha256":hash(content)
    })
}

#[test]
fn embedded_bundle_is_hash_verified_and_contains_only_compact_policy() {
    let knowledge = AgentKnowledge::load_embedded("v3.0.0").expect("embedded knowledge");
    assert_eq!(knowledge.manifest.agent_bundle_version, "v3.0.0");
    assert_eq!(
        knowledge.manifest.engine_contract_version,
        "tensoreval-parquet-v3"
    );
    assert_eq!(
        knowledge.manifest.eig_rs_commit,
        "9a01af9a60bcbb3b2814f08dc269456ac1399280"
    );
    assert_eq!(
        knowledge.manifest.sparrowzz_commit,
        "b3fc74c0ff1fd25deb4448edd3a9c0e8644bb3c1"
    );
    assert!(!knowledge.manifest.source_dirty);
    assert_eq!(knowledge.skill_index().count(), 6);
    assert!(knowledge.skill("result-analysis").is_some());
    assert!(!knowledge.system_prompt.contains("/Users/"));
    assert!(!knowledge.system_prompt.contains("TENSOREVAL_CWD"));
    assert!(!knowledge.system_prompt.contains("p_nom"));
    assert!(!knowledge.system_prompt.contains("API_KEY"));
}

#[test]
fn preflight_rejects_missing_tampered_or_unsafe_skills() {
    let content = "skill".to_string();
    let base_files = BTreeMap::from([("skills/a.md".to_string(), content.clone())]);

    let index =
        json!({"bundleVersion":"test","skills":[skill("a","skills/a.md","known_tool",&content)]});
    let (manifest, index_text, system, _) = fixture(index, base_files.clone());
    let missing =
        AgentKnowledge::load_from_parts("test", &manifest, &index_text, &system, BTreeMap::new())
            .unwrap_err();
    assert_eq!(missing.code(), "agent_skill_missing");

    let mut tampered = base_files.clone();
    tampered.insert("skills/a.md".to_string(), "changed".to_string());
    let error = AgentKnowledge::load_from_parts("test", &manifest, &index_text, &system, tampered)
        .unwrap_err();
    assert_eq!(error.code(), "agent_skill_hash_mismatch");

    for (index, expected) in [
        (
            json!({"bundleVersion":"test","skills":[skill("a","skills/a.md","known_tool",&content),skill("a","skills/a.md","known_tool",&content)]}),
            "duplicate_skill_id",
        ),
        (
            json!({"bundleVersion":"test","skills":[skill("a","skills/a.md","unknown",&content)]}),
            "unknown_required_tool",
        ),
        (
            json!({"bundleVersion":"test","skills":[skill("a","../escape.md","known_tool",&content)]}),
            "agent_skill_path_invalid",
        ),
    ] {
        let (manifest, index, system, files) = fixture(index, base_files.clone());
        let error =
            AgentKnowledge::load_from_parts("test", &manifest, &index, &system, files).unwrap_err();
        assert_eq!(error.code(), expected);
    }
}

#[test]
fn preflight_rejects_bundle_and_engine_contract_mismatch() {
    let content = "skill".to_string();
    let files = BTreeMap::from([("skills/a.md".to_string(), content.clone())]);
    let index =
        json!({"bundleVersion":"test","skills":[skill("a","skills/a.md","known_tool",&content)]});
    let (manifest, index, system, files) = fixture(index, files);
    assert_eq!(
        AgentKnowledge::load_from_parts("other", &manifest, &index, &system, files.clone())
            .unwrap_err()
            .code(),
        "agent_bundle_mismatch"
    );

    let mut index_value: Value = serde_json::from_str(&index).unwrap();
    index_value["skills"][0]["engineContractVersion"] = json!("wrong");
    let (manifest, index, system, files) = fixture(index_value, files);
    assert_eq!(
        AgentKnowledge::load_from_parts("test", &manifest, &index, &system, files)
            .unwrap_err()
            .code(),
        "engine_contract_mismatch"
    );
}
