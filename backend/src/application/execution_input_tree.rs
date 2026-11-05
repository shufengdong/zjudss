use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::adapters::NamedInput;
use crate::auth::CurrentUser;

use super::{ArtifactError, ArtifactService};

#[derive(Debug, Clone)]
pub(crate) struct ExecutionInputArtifact {
    pub alias: String,
    pub artifact_id: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutionInputSpec {
    pub script_artifact_id: i32,
    pub model_frames: Vec<ExecutionInputArtifact>,
    pub resources: Vec<ExecutionInputArtifact>,
}

#[derive(Debug)]
pub(crate) struct ExecutionInputTree {
    pub root: PathBuf,
    pub input_dir: PathBuf,
    pub result_dir: PathBuf,
    pub script_path: PathBuf,
    pub named_inputs: Vec<NamedInput>,
    keep: bool,
}

impl ExecutionInputTree {
    pub fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for ExecutionInputTree {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[derive(Debug)]
pub(crate) enum ExecutionInputError {
    Invalid(&'static str, String),
    Artifact(ArtifactError),
    Io(std::io::Error),
}

impl std::fmt::Display for ExecutionInputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(code, message) => write!(formatter, "{code}: {message}"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ExecutionInputError {}

pub(crate) async fn build_execution_input_tree(
    user: &CurrentUser,
    artifacts: &ArtifactService,
    data_dir: &Path,
    owner_kind: &str,
    owner_id: i32,
    spec: ExecutionInputSpec,
) -> Result<ExecutionInputTree, ExecutionInputError> {
    if !valid_segment(owner_kind) {
        return Err(ExecutionInputError::Invalid(
            "input_owner_invalid",
            "执行输入 owner kind 非法".into(),
        ));
    }
    let mut aliases = HashSet::new();
    for input in spec.model_frames.iter().chain(&spec.resources) {
        if !valid_alias(&input.alias) {
            return Err(ExecutionInputError::Invalid(
                "input_alias_invalid",
                format!("非法 input alias: {}", input.alias),
            ));
        }
        if !aliases.insert(input.alias.clone()) {
            return Err(ExecutionInputError::Invalid(
                "input_alias_duplicate",
                format!("重复 input alias: {}", input.alias),
            ));
        }
    }
    let root = data_dir.join(owner_kind).join(owner_id.to_string());
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(ExecutionInputError::Io)?;
    }
    let input_dir = root.join("input");
    let result_dir = root.join("result");
    std::fs::create_dir_all(&input_dir).map_err(ExecutionInputError::Io)?;
    std::fs::create_dir_all(&result_dir).map_err(ExecutionInputError::Io)?;
    let tree = ExecutionInputTree {
        root,
        input_dir,
        result_dir,
        script_path: PathBuf::from("script.rss"),
        named_inputs: Vec::new(),
        keep: false,
    };
    populate_tree(user, artifacts, tree, spec).await
}

async fn populate_tree(
    user: &CurrentUser,
    artifacts: &ArtifactService,
    mut tree: ExecutionInputTree,
    spec: ExecutionInputSpec,
) -> Result<ExecutionInputTree, ExecutionInputError> {
    copy_artifact(
        user,
        artifacts,
        spec.script_artifact_id,
        &tree.input_dir.join(&tree.script_path),
    )
    .await?;
    for (directory, input) in spec
        .model_frames
        .into_iter()
        .map(|value| ("model", value))
        .chain(spec.resources.into_iter().map(|value| ("inputs", value)))
    {
        let relative = PathBuf::from(directory).join(format!("{}.parquet", input.alias));
        copy_artifact(
            user,
            artifacts,
            input.artifact_id,
            &tree.input_dir.join(&relative),
        )
        .await?;
        tree.named_inputs.push(NamedInput {
            alias: input.alias,
            relative_path: relative,
        });
    }
    Ok(tree)
}

async fn copy_artifact(
    user: &CurrentUser,
    artifacts: &ArtifactService,
    artifact_id: i32,
    target: &Path,
) -> Result<(), ExecutionInputError> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(ExecutionInputError::Io)?;
    }
    let mut source = artifacts
        .open_owned(user.storage_scope_id, artifact_id)
        .await
        .map_err(ExecutionInputError::Artifact)?;
    let mut target = std::fs::File::create(target).map_err(ExecutionInputError::Io)?;
    std::io::copy(&mut source, &mut target).map_err(ExecutionInputError::Io)?;
    Ok(())
}

fn valid_alias(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|value| value.is_ascii_alphabetic() || value == '_')
        && chars.all(|value| value.is_ascii_alphanumeric() || value == '_')
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_'))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{ExecutionInputTree, valid_alias, valid_segment};

    #[test]
    fn rejects_unsafe_segments_and_aliases_before_tree_creation() {
        assert!(valid_segment("managed-workflows"));
        assert!(!valid_segment("../managed"));
        assert!(valid_alias("bus_1"));
        assert!(!valid_alias("../bus"));
        assert!(!valid_alias("bus/load"));
    }

    #[test]
    fn tree_drop_removes_private_execution_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("execution");
        std::fs::create_dir_all(root.join("input")).unwrap();
        std::fs::create_dir_all(root.join("result")).unwrap();
        let tree = ExecutionInputTree {
            root: root.clone(),
            input_dir: root.join("input"),
            result_dir: root.join("result"),
            script_path: PathBuf::from("script.rss"),
            named_inputs: Vec::new(),
            keep: false,
        };
        drop(tree);
        assert!(!root.exists());
    }
}
