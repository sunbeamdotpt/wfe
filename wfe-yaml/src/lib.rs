pub mod compiler;
pub mod error;
pub mod executors;
pub mod interpolation;
pub mod schema;
pub mod types;
pub mod validation;

use std::collections::HashMap;

use serde::de::Error as _;

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
///
/// Supports YAML 1.1 merge keys (`<<: *anchor`) via the `yaml-merge-keys`
/// crate. serde_yaml 0.9 implements YAML 1.2 which dropped merge keys;
/// we preprocess the YAML to resolve them before deserialization.
pub fn load_workflow_from_str(
    yaml: &str,
    config: &HashMap<String, serde_json::Value>,
) -> Result<Vec<CompiledWorkflow>, YamlWorkflowError> {
    // Interpolate variables.
    let interpolated = interpolation::interpolate(yaml, config)?;

    // Parse to a generic YAML value first, then resolve merge keys (<<:).
    // This adds YAML 1.1 merge key support on top of serde_yaml 0.9's YAML 1.2 parser.
    let raw_value: serde_yaml::Value = serde_yaml::from_str(&interpolated)?;
    let merged_value = yaml_merge_keys::merge_keys_serde(raw_value)
        .map_err(|e| YamlWorkflowError::Parse(serde_yaml::Error::custom(format!("merge key resolution failed: {e}"))))?;

    // Deserialize the merge-resolved value into our schema.
    let file: schema::YamlWorkflowFile = serde_yaml::from_value(merged_value)?;

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
