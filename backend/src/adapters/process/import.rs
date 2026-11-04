use std::collections::HashSet;

use super::ProcessEngine;
use super::execution::cleanup_result_dir;
use super::task_file::TemporaryTaskFile;
use crate::adapters::{
    AdapterError, CaseProjectionSpec, EXECUTION_IMPORT_TABLES, ErrorCategory, ImportProfile,
    ImportSpec, ImportedModel, ProjectedCase, RunMode, RunSpec, STATIC_IMPORT_OUTPUTS,
};

impl ProcessEngine {
    pub(super) async fn import_model_process(
        &self,
        spec: ImportSpec,
    ) -> Result<ImportedModel, AdapterError> {
        let (entry, profile) = crate::adapters::import::resolve_entry(&spec)?;
        let (function, output_names): (&str, &[&str]) = match profile {
            ImportProfile::StaticDss => ("import_dss", STATIC_IMPORT_OUTPUTS.as_slice()),
            ImportProfile::SingleSolveExecution => {
                ("import_dss_execution", EXECUTION_IMPORT_TABLES.as_slice())
            }
        };
        let entry = entry.to_str().ok_or_else(|| {
            AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "DSS 入口不是 UTF-8",
            )
        })?;
        let entry = entry.replace('\\', "/");
        if entry
            .chars()
            .any(|character| character == '"' || character.is_control())
        {
            return Err(AdapterError::new(
                "input_path_invalid",
                ErrorCategory::Model,
                "DSS 入口包含不安全字符",
            ));
        }
        let wrapper = TemporaryTaskFile::create(
            &spec.input_tree,
            ".zjudss-model-entry-",
            "dss",
            format!("Redirect \"{entry}\"\n").as_bytes(),
            "model_import_script_failed",
        )?;
        let wrapper_name = wrapper
            .relative_path()
            .to_str()
            .expect("generated wrapper name is ASCII");
        let wrapper_literal = serde_json::to_string(wrapper_name).map_err(|error| {
            AdapterError::new(
                "model_import_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        let outputs = output_names.join(", ");
        let script = TemporaryTaskFile::create(
            &spec.input_tree,
            ".zjudss-model-import-",
            "rss",
            format!("[{outputs}] = {function}({wrapper_literal});\n").as_bytes(),
            "model_import_script_failed",
        )?;
        let script_path = script.relative_path().to_path_buf();
        let executed = self
            .execute(
                RunSpec {
                    mode: RunMode::Dataframe,
                    input_tree: spec.input_tree,
                    script_path,
                    named_inputs: Vec::new(),
                    result_dir: spec.result_dir,
                    timeout: spec.timeout,
                },
                &[RunMode::Dataframe],
            )
            .await;
        let cleanup = script.remove().and_then(|()| wrapper.remove());
        let output = combine_execution_and_cleanup(executed, cleanup, "导入临时文件清理失败")?;
        let datasets = if profile == ImportProfile::SingleSolveExecution {
            let model_names = STATIC_IMPORT_OUTPUTS.into_iter().collect::<HashSet<_>>();
            output
                .datasets
                .into_iter()
                .filter(|dataset| model_names.contains(dataset.name.as_str()))
                .collect()
        } else {
            output.datasets
        };
        Ok(ImportedModel {
            profile,
            dataframes: crate::adapters::import::order_model_outputs(datasets)?,
            diagnostics: serde_json::json!({
                "importMethod": function,
                "tableCount": STATIC_IMPORT_OUTPUTS.len()
            }),
        })
    }

    pub(super) async fn project_case_process(
        &self,
        spec: CaseProjectionSpec,
    ) -> Result<ProjectedCase, AdapterError> {
        spec.validate()?;
        let source = spec
            .source_entry
            .to_str()
            .expect("validated UTF-8 case source")
            .replace('\\', "/");
        let source_literal = serde_json::to_string(&source).map_err(|error| {
            AdapterError::new(
                "case_projection_script_failed",
                ErrorCategory::Internal,
                error.to_string(),
            )
        })?;
        let outputs = spec
            .outputs
            .iter()
            .map(|output| output.alias.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let script = TemporaryTaskFile::create(
            &spec.input_tree,
            ".zjudss-case-projection-",
            "rss",
            format!("[{outputs}] = {}({source_literal});\n", spec.projector).as_bytes(),
            "case_projection_script_failed",
        )?;
        let script_path = script.relative_path().to_path_buf();
        let expected = spec.outputs;
        let result_dir = spec.result_dir.clone();
        let executed = self
            .execute(
                RunSpec {
                    mode: RunMode::Dataframe,
                    input_tree: spec.input_tree,
                    script_path,
                    named_inputs: Vec::new(),
                    result_dir: spec.result_dir,
                    timeout: spec.timeout,
                },
                &[RunMode::Dataframe],
            )
            .await;
        let cleanup = script.remove();
        let output =
            match combine_execution_and_cleanup(executed, cleanup, "案例投影临时脚本清理失败")
            {
                Ok(output) => output,
                Err(error) => {
                    if error.code == "case_projection_script_failed" {
                        cleanup_result_dir(&result_dir);
                    }
                    return Err(error);
                }
            };
        match crate::adapters::import::order_projection_outputs(output.datasets, &expected) {
            Ok(projected) => Ok(projected),
            Err(error) => {
                cleanup_result_dir(&result_dir);
                Err(error)
            }
        }
    }
}

fn combine_execution_and_cleanup(
    executed: Result<crate::adapters::EngineRunOutput, AdapterError>,
    cleanup: Result<(), AdapterError>,
    cleanup_context: &str,
) -> Result<crate::adapters::EngineRunOutput, AdapterError> {
    match (executed, cleanup) {
        (Ok(output), Ok(())) => Ok(output),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Ok(())) => Err(error),
        (Err(mut error), Err(cleanup_error)) => {
            error.message = format!(
                "{}; {cleanup_context}: {}",
                error.message, cleanup_error.message
            );
            Err(error)
        }
    }
}
