use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::adapters::SimEngine;
use crate::application::study_planning::ReadyStudyPlan;
use crate::auth::CurrentUser;
use crate::db::models::{NewScriptResource, ScriptResource};
use crate::db::repositories::{RepoError, ScriptRepo, VersionRepo};
use crate::storage::StageLimits;

use super::{
    ArtifactError, ArtifactMetadata, ArtifactService, EventBus, EventWriter, MessageRepo,
    ms_to_iso, now_ms,
};

const SCRIPT_PREVIEW_LINES: usize = 20;
const SCRIPT_MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSource {
    Upload,
    Agent,
    Manual,
}
impl ScriptSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Agent => "agent",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CreateScriptRequest {
    pub name: String,
    pub content: String,
    pub source: ScriptSource,
    pub agent_bundle_version: Option<String>,
    pub agent_skill_refs: Vec<Value>,
    pub study_plan: Option<ReadyStudyPlan>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptSummary {
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub source_kind: String,
    pub content_hash: String,
    pub validation_status: String,
    pub required_aliases: Vec<String>,
    pub outputs: Vec<String>,
    pub diagnostics: Value,
    pub preview: String,
    pub preview_truncated: bool,
    pub can_run: bool,
    pub disabled_reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDetail {
    #[serde(flatten)]
    pub summary: ScriptSummary,
    pub content: String,
}

#[derive(Debug)]
pub enum ScriptError {
    NotFound,
    Invalid(String),
    Artifact(ArtifactError),
    Repository(RepoError),
    Io(std::io::Error),
    StudyPlan { code: &'static str, message: String },
}

impl ScriptError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::StudyPlan { code, .. } => code,
            _ => "script_error",
        }
    }

    fn study_plan(code: &'static str, message: impl Into<String>) -> Self {
        Self::StudyPlan {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::Invalid(v) => write!(f, "{v}"),
            Self::Artifact(v) => write!(f, "{v}"),
            Self::Repository(v) => write!(f, "{v}"),
            Self::Io(v) => write!(f, "{v}"),
            Self::StudyPlan { message, .. } => write!(f, "{message}"),
        }
    }
}
impl std::error::Error for ScriptError {}
impl From<ArtifactError> for ScriptError {
    fn from(v: ArtifactError) -> Self {
        Self::Artifact(v)
    }
}
impl From<RepoError> for ScriptError {
    fn from(v: RepoError) -> Self {
        if matches!(v, RepoError::NotFound) {
            Self::NotFound
        } else {
            Self::Repository(v)
        }
    }
}

#[derive(Clone)]
pub struct ScriptService {
    repo: ScriptRepo,
    artifacts: ArtifactService,
    _engine: Arc<dyn SimEngine>,
    _data_dir: PathBuf,
    messages: MessageRepo,
    events: EventBus,
    versions: VersionRepo,
    engine_snapshot_id: String,
}

pub struct ScriptServiceDependencies {
    pub repo: ScriptRepo,
    pub artifacts: ArtifactService,
    pub engine: Arc<dyn SimEngine>,
    pub data_dir: PathBuf,
    pub messages: MessageRepo,
    pub events: EventBus,
    pub versions: VersionRepo,
    pub engine_snapshot_id: String,
}

impl ScriptService {
    pub fn new(dependencies: ScriptServiceDependencies) -> Self {
        Self {
            repo: dependencies.repo,
            artifacts: dependencies.artifacts,
            _engine: dependencies.engine,
            _data_dir: dependencies.data_dir,
            messages: dependencies.messages,
            events: dependencies.events,
            versions: dependencies.versions,
            engine_snapshot_id: dependencies.engine_snapshot_id,
        }
    }

    pub async fn create(
        &self,
        user: &CurrentUser,
        session_id: i32,
        request: CreateScriptRequest,
    ) -> Result<ScriptSummary, ScriptError> {
        let name = request.name.trim();
        if name.is_empty() || name.len() > 128 || !name.to_ascii_lowercase().ends_with(".rss") {
            return Err(ScriptError::Invalid(
                "脚本名称必须是 1 到 128 个字符的 .rss 文件名".into(),
            ));
        }
        if request.content.is_empty() || request.content.len() as u64 > SCRIPT_MAX_BYTES {
            return Err(ScriptError::Invalid("脚本内容为空或超过大小限制".into()));
        }
        let mut required_aliases = Vec::new();
        let mut outputs = Vec::new();
        let mut agent_skill_refs = request.agent_skill_refs;
        if request.source == ScriptSource::Agent && request.study_plan.is_some() {
            let plan = request.study_plan.as_ref().ok_or_else(|| {
                ScriptError::study_plan(
                    "study_plan_required",
                    "受管研究脚本必须提供本次请求生成的 studyPlanId",
                )
            })?;
            if plan.engine_snapshot_id != self.engine_snapshot_id {
                return Err(ScriptError::study_plan(
                    "study_plan_stale",
                    "引擎资源快照已变化，请重新规划研究",
                ));
            }
            let workspace = self
                .versions
                .workspace(&user.id, session_id, now_ms())
                .await
                .map_err(|error| {
                    ScriptError::Repository(match error {
                        crate::db::repositories::VersionRepoError::Repository(error) => error,
                        _ => RepoError::InvalidState("workspace state changed".into()),
                    })
                })?;
            let active_matches = workspace.active.as_ref().is_some_and(|active| {
                active.version.id == plan.model_version_id
                    && active.version.content_hash == plan.model_content_hash
            });
            if !active_matches {
                return Err(ScriptError::study_plan(
                    "study_plan_stale",
                    "active 模型已变化，请重新规划研究",
                ));
            }
            if !agent_skill_refs
                .iter()
                .any(|reference| reference["id"] == "study-planning")
            {
                agent_skill_refs.push(serde_json::json!({"id":"study-planning"}));
            }
            if !agent_skill_refs
                .iter()
                .any(|reference| reference["id"] == "rss-generation")
            {
                agent_skill_refs.push(serde_json::json!({"id":"rss-generation"}));
            }
            let plan_json = crate::application::canonical_json(
                &serde_json::to_value(plan).expect("ReadyStudyPlan serializes"),
            );
            agent_skill_refs.push(serde_json::json!({
                "kind": "study-plan",
                "sha256": hex::encode(Sha256::digest(plan_json.as_bytes())),
                "modelContentHash": plan.model_content_hash,
                "engineSnapshotId": plan.engine_snapshot_id,
                "function": plan.function,
            }));
            required_aliases = plan.input_aliases.clone();
            outputs = plan.results.clone();
        }
        let staged = self
            .artifacts
            .store()
            .stage(
                std::io::Cursor::new(request.content.as_bytes()),
                StageLimits {
                    max_bytes: SCRIPT_MAX_BYTES,
                },
            )
            .map_err(ArtifactError::from)?;
        let artifact = self
            .artifacts
            .commit(
                user.storage_scope_id,
                staged,
                ArtifactMetadata {
                    format: "rss".into(),
                    media_type: Some("text/plain".into()),
                },
            )
            .await?;
        let record = NewScriptResource {
            session_id,
            artifact_id: artifact.id,
            name: name.to_string(),
            source_kind: request.source.as_str().into(),
            content_hash: artifact.sha256,
            // `valid` now means accepted for execution; RSS syntax and runtime
            // viability are intentionally deferred to tensoreval.
            validation_status: "valid".into(),
            required_aliases_json: serde_json::to_string(&required_aliases).unwrap(),
            outputs_json: serde_json::to_string(&outputs).unwrap(),
            diagnostics_json: serde_json::json!({
                "validation": "deferred_to_tensoreval"
            })
            .to_string(),
            agent_bundle_version: request.agent_bundle_version,
            agent_skill_refs_json: serde_json::to_string(&agent_skill_refs).unwrap(),
            created_at_ms: now_ms(),
        };
        let script = match self.repo.create(&user.id, record).await {
            Ok(v) => v,
            Err(error) => {
                let _ = self
                    .artifacts
                    .release(user.storage_scope_id, artifact.id)
                    .await;
                return Err(error.into());
            }
        };
        let summary = self.summary(user, &script, &request.content).await?;
        EventWriter::new(
            user.id.clone(),
            session_id,
            self.messages.clone(),
            self.events.clone(),
        )
        .script(serde_json::to_value(&summary).unwrap_or_default())
        .await?;
        Ok(summary)
    }

    pub async fn list(
        &self,
        user: &CurrentUser,
        session_id: i32,
    ) -> Result<Vec<ScriptSummary>, ScriptError> {
        let mut values = Vec::new();
        for script in self.repo.list(&user.id, session_id).await? {
            let content = self.read_content(user, &script).await?;
            values.push(self.summary(user, &script, &content).await?);
        }
        Ok(values)
    }

    pub async fn detail(&self, user: &CurrentUser, id: i32) -> Result<ScriptDetail, ScriptError> {
        let script = self.repo.find(&user.id, id).await?;
        let content = self.read_content(user, &script).await?;
        let summary = self.summary(user, &script, &content).await?;
        Ok(ScriptDetail { summary, content })
    }

    pub async fn session_artifact_ids(
        &self,
        user: &CurrentUser,
        session_id: i32,
    ) -> Result<Vec<i32>, ScriptError> {
        self.repo
            .artifact_ids(&user.id, session_id)
            .await
            .map_err(Into::into)
    }

    async fn read_content(
        &self,
        user: &CurrentUser,
        script: &ScriptResource,
    ) -> Result<String, ScriptError> {
        let artifact = self
            .artifacts
            .find_owned(user.storage_scope_id, script.artifact_id)
            .await?;
        let path = self
            .artifacts
            .store()
            .resolve(&artifact.storage_path)
            .map_err(ArtifactError::from)?;
        std::fs::read_to_string(path).map_err(ScriptError::Io)
    }

    async fn summary(
        &self,
        user: &CurrentUser,
        script: &ScriptResource,
        content: &str,
    ) -> Result<ScriptSummary, ScriptError> {
        let required_aliases: Vec<String> =
            serde_json::from_str(&script.required_aliases_json).unwrap_or_default();
        let outputs: Vec<String> = serde_json::from_str(&script.outputs_json).unwrap_or_default();
        let active = self
            .repo
            .active_aliases(&user.id, script.session_id)
            .await?;
        let (can_run, disabled_reason) = if script.validation_status == "invalid" {
            (false, Some("script_static_invalid".into()))
        } else if let Some(aliases) = active {
            let missing = required_aliases
                .iter()
                .filter(|required| !aliases.contains(required))
                .cloned()
                .collect::<Vec<_>>();
            if missing.is_empty() {
                (true, None)
            } else {
                (
                    false,
                    Some(format!("missing_model_aliases:{}", missing.join(","))),
                )
            }
        } else {
            (false, Some("no_active_model".into()))
        };
        let lines = content.lines().collect::<Vec<_>>();
        let preview_truncated = lines.len() > SCRIPT_PREVIEW_LINES;
        let preview = lines
            .into_iter()
            .take(SCRIPT_PREVIEW_LINES)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ScriptSummary {
            id: script.id.to_string(),
            session_id: script.session_id.to_string(),
            name: script.name.clone(),
            source_kind: script.source_kind.clone(),
            content_hash: script.content_hash.clone(),
            validation_status: script.validation_status.clone(),
            required_aliases,
            outputs,
            diagnostics: serde_json::from_str(&script.diagnostics_json).unwrap_or_default(),
            preview,
            preview_truncated,
            can_run,
            disabled_reason,
            created_at: ms_to_iso(script.created_at_ms),
        })
    }
}
