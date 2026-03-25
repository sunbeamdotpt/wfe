pub mod compiler;
pub mod error;
pub mod executors;
pub mod interpolation;
pub mod schema;
pub mod validation;

use std::collections::HashMap;

use crate::compiler::CompiledWorkflow;
use crate::error::YamlWorkflowError;

/// Load a workflow from a YAML file path, applying variable interpolation.
pub fn load_workflow(
    path: &std::path::Path,
    config: &HashMap<String, serde_json::Value>,
) -> Result<CompiledWorkflow, YamlWorkflowError> {
    let yaml = std::fs::read_to_string(path)?;
    load_workflow_from_str(&yaml, config)
}

/// Load a workflow from a YAML string, applying variable interpolation.
pub fn load_workflow_from_str(
    yaml: &str,
    config: &HashMap<String, serde_json::Value>,
) -> Result<CompiledWorkflow, YamlWorkflowError> {
    // Interpolate variables.
    let interpolated = interpolation::interpolate(yaml, config)?;

    // Parse YAML.
    let workflow: schema::YamlWorkflow = serde_yaml::from_str(&interpolated)?;

    // Validate.
    validation::validate(&workflow.workflow)?;

    // Compile.
    compiler::compile(&workflow.workflow)
}
