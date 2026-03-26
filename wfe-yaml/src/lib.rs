pub mod compiler;
pub mod error;
pub mod executors;
pub mod interpolation;
pub mod schema;
pub mod types;
pub mod validation;

use std::collections::HashMap;

use crate::compiler::CompiledWorkflow;
use crate::error::YamlWorkflowError;

/// Load workflows from a YAML file path, applying variable interpolation.
/// Returns a Vec of compiled workflows (supports multi-workflow files).
pub fn load_workflow(
    path: &std::path::Path,
    config: &HashMap<String, serde_json::Value>,
) -> Result<CompiledWorkflow, YamlWorkflowError> {
    let yaml = std::fs::read_to_string(path)?;
    load_single_workflow_from_str(&yaml, config)
}

/// Load workflows from a YAML string, applying variable interpolation.
/// Returns a Vec of compiled workflows (supports multi-workflow files).
pub fn load_workflow_from_str(
    yaml: &str,
    config: &HashMap<String, serde_json::Value>,
) -> Result<Vec<CompiledWorkflow>, YamlWorkflowError> {
    // Interpolate variables.
    let interpolated = interpolation::interpolate(yaml, config)?;

    // Parse YAML as multi-workflow file.
    let file: schema::YamlWorkflowFile = serde_yaml::from_str(&interpolated)?;

    let specs = resolve_workflow_specs(file)?;

    // Validate (multi-workflow validation includes per-workflow + cross-references).
    validation::validate_multi(&specs)?;

    // Compile each workflow.
    let mut results = Vec::with_capacity(specs.len());
    for spec in &specs {
        results.push(compiler::compile(spec)?);
    }

    Ok(results)
}

/// Load a single workflow from a YAML string. Returns an error if the file
/// contains more than one workflow. This is a backward-compatible convenience
/// function.
pub fn load_single_workflow_from_str(
    yaml: &str,
    config: &HashMap<String, serde_json::Value>,
) -> Result<CompiledWorkflow, YamlWorkflowError> {
    let mut workflows = load_workflow_from_str(yaml, config)?;
    if workflows.len() != 1 {
        return Err(YamlWorkflowError::Validation(format!(
            "Expected single workflow, got {}",
            workflows.len()
        )));
    }
    Ok(workflows.remove(0))
}

/// Resolve a YamlWorkflowFile into a list of WorkflowSpecs.
fn resolve_workflow_specs(
    file: schema::YamlWorkflowFile,
) -> Result<Vec<schema::WorkflowSpec>, YamlWorkflowError> {
    match (file.workflow, file.workflows) {
        (Some(single), None) => Ok(vec![single]),
        (None, Some(multi)) => {
            if multi.is_empty() {
                return Err(YamlWorkflowError::Validation(
                    "workflows list is empty".to_string(),
                ));
            }
            Ok(multi)
        }
        (Some(_), Some(_)) => Err(YamlWorkflowError::Validation(
            "Cannot specify both 'workflow' and 'workflows' in the same file".to_string(),
        )),
        (None, None) => Err(YamlWorkflowError::Validation(
            "Must specify either 'workflow' or 'workflows'".to_string(),
        )),
    }
}
