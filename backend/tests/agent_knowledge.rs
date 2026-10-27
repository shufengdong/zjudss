mod common;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zjudss::agent::knowledge::AgentKnowledge;
use zjudss::agent::prompt::PromptAssembler;

fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn fixture(
    index: Value,
    files: BTreeMap<String, String>,
) -> (String, String, String, BTreeMap<String, String>) {
    let index = index.to_string();
    let system = "Stable system rules".to_string();
    let manifest = json!({
        "agentBundleVersion":"test", "engineContractVersion":"contract-1",
        "systemSha256":hash(&system),
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
    let knowledge = AgentKnowledge::load_embedded("v3.3.3").expect("embedded knowledge");
    assert_eq!(knowledge.manifest.agent_bundle_version, "v3.3.3");
    assert_eq!(
        knowledge.manifest.engine_contract_version,
        "tensoreval-parquet-v3"
    );
    let manifest = serde_json::to_value(&knowledge.manifest).unwrap();
    for removed in [
        "eigRsCommit",
        "sparrowzzCommit",
        "sourceDirty",
        "catalogSchemaVersion",
        "compatibleCatalogSchemaVersions",
        "catalogs",
        "docsTreeSha256",
        "examplesSourceSha256",
    ] {
        assert!(
            manifest.get(removed).is_none(),
            "stale engine pin: {removed}"
        );
    }
    assert_eq!(knowledge.skill_index().count(), 6);
    assert!(knowledge.skill("result-analysis").is_some());
    let study_skill = knowledge.skill("study-planning").unwrap();
    assert_eq!(study_skill.metadata.version, "1.3.0");
    let study_planning = &study_skill.content;
    assert!(study_planning.contains("search_study_facades"));
    assert!(study_planning.contains("prepare_study"));
    assert!(study_planning.contains("explicitFunction"));
    assert!(study_planning.contains("多个资源时必须由用户选择"));
    assert!(study_planning.contains("Newton 适用于径向和非径向网络"));
    assert!(!study_planning.contains("plan_study"));
    let rss_skill = knowledge.skill("rss-generation").unwrap();
    assert_eq!(rss_skill.metadata.version, "1.3.0");
    let rss_generation = &rss_skill.content;
    assert!(rss_generation.contains("函数结果绑定本身就是输出"));
    assert!(rss_generation.contains("未被后续节点消费的纯标量"));
    assert!(rss_generation.contains("prepare_study"));
    assert!(rss_generation.contains("不得用通用脚本接口生成"));
    assert!(rss_generation.contains("只能包含变量声明"));
    assert!(rss_generation.contains("函数调用后不要追加"));
    assert!(rss_generation.contains("文件读取"));
    assert!(!study_planning.contains("bus, line, linecode"));
    assert!(!rss_generation.contains("bus, line, linecode"));
    assert!(knowledge.system_prompt.contains("inspect_workspace"));
    assert!(knowledge.system_prompt.contains("search_study_facades"));
    assert!(knowledge.system_prompt.contains("prepare_study"));
    assert!(knowledge.system_prompt.contains("确认并生成脚本"));
    assert!(!knowledge.system_prompt.contains("plan_study"));
    assert!(!knowledge.system_prompt.contains("bus, line, linecode"));
    assert!(!knowledge.system_prompt.contains("/Users/"));
    assert!(!knowledge.system_prompt.contains("SPARROWZZ_DIR"));
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

#[tokio::test]
async fn prompt_publishes_independent_catalog_versions_and_facade_inventory() {
    let app = common::TestApp::spawn().await;
    let prompt = PromptAssembler::assemble(&app.state.agent_knowledge, &app.state.engine_snapshot);

    assert!(prompt.contains("catalogSchemaVersions"));
    assert!(prompt.contains("studyFacades"));
    assert!(prompt.contains("\"functions\":2"));
    assert!(prompt.contains("\"studyFacades\":1"));
    assert!(prompt.contains("\"dataframes\":3"));
    assert!(prompt.contains("\"dtypeGrammar\":1"));
    assert!(!prompt.contains("\"catalogSchemaVersion\":"));
}
