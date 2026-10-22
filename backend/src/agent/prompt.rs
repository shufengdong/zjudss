use serde_json::json;

use crate::agent::knowledge::AgentKnowledge;
use crate::engine_resources::EngineResourceSnapshot;

/// Builds the bounded context that is injected on every run. Full skill and
/// engine document bodies remain behind ID based tools.
pub struct PromptAssembler;

impl PromptAssembler {
    pub fn assemble(knowledge: &AgentKnowledge, resources: &EngineResourceSnapshot) -> String {
        let skills: Vec<_> = knowledge
            .skill_index()
            .map(|skill| {
                json!({
                    "id": skill.id,
                    "version": skill.version,
                    "description": skill.description,
                    "triggers": skill.triggers,
                    "requiredTools": skill.required_tools,
                    "sha256": skill.sha256,
                })
            })
            .collect();
        let summary = json!({
            "engineResourceSnapshot": resources.id,
            "engineContractVersion": resources.contract_version,
            "catalogSchemaVersion": resources.catalog_schema_version,
            "documentSections": resources.knowledge.sections.len(),
            "examples": resources.examples.len(),
        });
        format!(
            "{}\n\n# Skill index\n{}\n\n# Engine resources\n{}",
            knowledge.system_prompt,
            serde_json::to_string(&skills).expect("skill index serializable"),
            serde_json::to_string(&summary).expect("resource summary serializable"),
        )
    }
}
