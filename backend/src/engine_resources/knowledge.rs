use sha2::{Digest, Sha256};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocSection {
    pub doc_id: String,
    pub section_id: String,
    pub hash: String,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocSearchHit {
    pub doc_id: String,
    pub section_id: String,
    pub hash: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone)]
pub struct EngineKnowledge {
    pub sections: Vec<DocSection>,
}

impl EngineKnowledge {
    pub fn from_documents(documents: &[(String, String)]) -> Self {
        let mut sections = Vec::new();
        for (path, content) in documents {
            let doc_id = stable_id("doc", path.as_bytes());
            let mut title = path.clone();
            let mut buffer = String::new();
            let mut ordinal = 0_u32;
            for line in content.lines() {
                if line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ") {
                    if !buffer.trim().is_empty() {
                        push_section(&mut sections, &doc_id, ordinal, &title, &buffer);
                        ordinal += 1;
                        buffer.clear();
                    }
                    title = line.trim_start_matches('#').trim().to_string();
                } else {
                    buffer.push_str(line);
                    buffer.push('\n');
                }
            }
            if !buffer.trim().is_empty() || ordinal == 0 {
                push_section(&mut sections, &doc_id, ordinal, &title, &buffer);
            }
        }
        Self { sections }
    }

    pub fn search(&self, query: &str, max_chars: usize) -> Vec<DocSearchHit> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|term| term.to_lowercase())
            .collect();
        if terms.is_empty() || max_chars == 0 {
            return Vec::new();
        }
        let mut ranked: Vec<(usize, &DocSection)> = self
            .sections
            .iter()
            .filter_map(|section| {
                let haystack = format!("{}\n{}", section.title, section.content).to_lowercase();
                let score = terms
                    .iter()
                    .filter(|term| haystack.contains(term.as_str()))
                    .count();
                (score > 0).then_some((score, section))
            })
            .collect();
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.section_id.cmp(&right.1.section_id))
        });
        let mut remaining = max_chars;
        let mut hits = Vec::new();
        for (_, section) in ranked {
            if remaining == 0 {
                break;
            }
            let snippet: String = section.content.chars().take(remaining.min(320)).collect();
            remaining = remaining.saturating_sub(snippet.chars().count());
            hits.push(DocSearchHit {
                doc_id: section.doc_id.clone(),
                section_id: section.section_id.clone(),
                hash: section.hash.clone(),
                title: section.title.clone(),
                snippet,
            });
        }
        hits
    }

    pub fn read(&self, section_id: &str, max_chars: usize) -> Option<DocSection> {
        self.sections
            .iter()
            .find(|section| section.section_id == section_id)
            .map(|section| {
                let mut section = section.clone();
                section.content = section.content.chars().take(max_chars).collect();
                section
            })
    }
}

fn push_section(
    sections: &mut Vec<DocSection>,
    doc_id: &str,
    ordinal: u32,
    title: &str,
    content: &str,
) {
    let identity = format!("{doc_id}:{ordinal}:{title}");
    sections.push(DocSection {
        doc_id: doc_id.to_string(),
        section_id: stable_id("sec", identity.as_bytes()),
        hash: hex::encode(Sha256::digest(content.as_bytes())),
        title: title.to_string(),
        content: content.trim().to_string(),
    });
}

fn stable_id(prefix: &str, bytes: &[u8]) -> String {
    let hash = hex::encode(Sha256::digest(bytes));
    format!("{prefix}_{}", &hash[..20])
}
