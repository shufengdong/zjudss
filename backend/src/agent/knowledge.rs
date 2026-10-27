use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const MANIFEST: &str = include_str!("../../resources/agent/v4/manifest.json");
const SKILL_INDEX: &str = include_str!("../../resources/agent/v4/skill-index.json");
const SYSTEM: &str = include_str!("../../resources/agent/v4/system.md");

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentManifest {
    pub agent_bundle_version: String,
    pub engine_contract_version: String,
    pub system_sha256: String,
    pub skill_index_sha256: String,
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillIndex {
    bundle_version: String,
    skills: Vec<SkillMetadata>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMetadata {
    pub id: String,
    pub version: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub required_tools: Vec<String>,
    pub engine_contract_version: String,
    pub sources: Vec<String>,
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct AgentSkill {
    pub metadata: SkillMetadata,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct AgentKnowledge {
    pub manifest: AgentManifest,
    pub system_prompt: String,
    skills: BTreeMap<String, AgentSkill>,
}

#[derive(Debug)]
pub struct KnowledgeError {
    code: &'static str,
    message: String,
}

impl KnowledgeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for KnowledgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for KnowledgeError {}

impl AgentKnowledge {
    pub fn load_embedded(bundle_version: &str) -> Result<Self, KnowledgeError> {
        let files = BTreeMap::from([
            (
                "skills/model-intake.md".to_string(),
                include_str!("../../resources/agent/v4/skills/model-intake.md").to_string(),
            ),
            (
                "skills/study-planning.md".to_string(),
                include_str!("../../resources/agent/v4/skills/study-planning.md").to_string(),
            ),
            (
                "skills/result-analysis.md".to_string(),
                include_str!("../../resources/agent/v4/skills/result-analysis.md").to_string(),
            ),
            (
                "skills/comparison.md".to_string(),
                include_str!("../../resources/agent/v4/skills/comparison.md").to_string(),
            ),
            (
                "skills/troubleshooting.md".to_string(),
                include_str!("../../resources/agent/v4/skills/troubleshooting.md").to_string(),
            ),
        ]);
        Self::load_from_parts(bundle_version, MANIFEST, SKILL_INDEX, SYSTEM, files)
    }

    pub fn load_from_parts(
        bundle_version: &str,
        manifest_json: &str,
        skill_index_json: &str,
        system_prompt: &str,
        files: BTreeMap<String, String>,
    ) -> Result<Self, KnowledgeError> {
        let manifest: AgentManifest = serde_json::from_str(manifest_json)
            .map_err(|error| KnowledgeError::new("agent_manifest_invalid", error.to_string()))?;
        let index: SkillIndex = serde_json::from_str(skill_index_json)
            .map_err(|error| KnowledgeError::new("agent_skill_index_invalid", error.to_string()))?;
        if manifest.agent_bundle_version != bundle_version || index.bundle_version != bundle_version
        {
            return Err(KnowledgeError::new(
                "agent_bundle_mismatch",
                "请求版本与资源版本不一致",
            ));
        }
        verify_hash(
            "agent_system_hash_mismatch",
            system_prompt.as_bytes(),
            &manifest.system_sha256,
        )?;
        verify_hash(
            "agent_skill_index_hash_mismatch",
            skill_index_json.as_bytes(),
            &manifest.skill_index_sha256,
        )?;
        let allowed_tools: HashSet<&str> =
            manifest.allowed_tools.iter().map(String::as_str).collect();
        let mut ids = HashSet::new();
        let mut skills = BTreeMap::new();
        for metadata in index.skills {
            if !ids.insert(metadata.id.clone()) {
                return Err(KnowledgeError::new("duplicate_skill_id", metadata.id));
            }
            if metadata.engine_contract_version != manifest.engine_contract_version {
                return Err(KnowledgeError::new("engine_contract_mismatch", metadata.id));
            }
            if metadata
                .required_tools
                .iter()
                .any(|tool| !allowed_tools.contains(tool.as_str()))
            {
                return Err(KnowledgeError::new("unknown_required_tool", metadata.id));
            }
            let path = Path::new(&metadata.path);
            if path.is_absolute()
                || !metadata.path.starts_with("skills/")
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(KnowledgeError::new(
                    "agent_skill_path_invalid",
                    metadata.path,
                ));
            }
            let content = files
                .get(&metadata.path)
                .ok_or_else(|| KnowledgeError::new("agent_skill_missing", &metadata.path))?;
            verify_hash(
                "agent_skill_hash_mismatch",
                content.as_bytes(),
                &metadata.sha256,
            )?;
            skills.insert(
                metadata.id.clone(),
                AgentSkill {
                    metadata,
                    content: content.clone(),
                },
            );
        }
        Ok(Self {
            manifest,
            system_prompt: system_prompt.to_string(),
            skills,
        })
    }

    pub fn skill(&self, id: &str) -> Option<&AgentSkill> {
        self.skills.get(id)
    }

    pub fn skill_index(&self) -> impl Iterator<Item = &SkillMetadata> {
        self.skills.values().map(|skill| &skill.metadata)
    }
}

fn verify_hash(code: &'static str, bytes: &[u8], expected: &str) -> Result<(), KnowledgeError> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(KnowledgeError::new(
            code,
            format!("期望 {expected}，实际 {actual}"),
        ))
    }
}
