pub mod compiler;
pub mod error;
pub mod executors;
pub mod interpolation;
pub mod schema;
pub mod types;
pub mod validation;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;
use serde::de::Error as _;

use crate::compiler::CompiledWorkflow;
use crate::error::YamlWorkflowError;

/// Top-level YAML file with optional includes.
#[derive(Deserialize)]
pub struct YamlWorkflowFileWithIncludes {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(flatten)]
    pub file: schema::YamlWorkflowFile,
}

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
    let merged_value = yaml_merge_keys::merge_keys_serde(raw_value).map_err(|e| {
        YamlWorkflowError::Parse(serde_yaml::Error::custom(format!(
            "merge key resolution failed: {e}"
        )))
    })?;

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

/// Load workflows from a YAML string, resolving `include:` paths relative to `base_path`.
///
/// Processing:
/// 1. Parse the main YAML to get `include:` paths.
/// 2. For each include path, load and parse that YAML file.
/// 3. Merge workflow specs from included files into the main file's specs.
/// 4. Main file's workflows take precedence over included ones (by ID).
/// 5. Proceed with normal validation + compilation.
pub fn load_workflow_with_includes(
    yaml: &str,
    base_path: &Path,
    config: &HashMap<String, serde_json::Value>,
) -> Result<Vec<CompiledWorkflow>, YamlWorkflowError> {
    let mut visited = HashSet::new();
    let canonical = base_path
        .canonicalize()
        .unwrap_or_else(|_| base_path.to_path_buf());
    visited.insert(canonical.to_string_lossy().to_string());

    let interpolated = interpolation::interpolate(yaml, config)?;
    let raw_value: serde_yaml::Value = serde_yaml::from_str(&interpolated)?;
    let merged_value = yaml_merge_keys::merge_keys_serde(raw_value).map_err(|e| {
        YamlWorkflowError::Parse(serde_yaml::Error::custom(format!(
            "merge key resolution failed: {e}"
        )))
    })?;

    let with_includes: YamlWorkflowFileWithIncludes = serde_yaml::from_value(merged_value)?;

    let mut main_specs = resolve_workflow_specs(with_includes.file)?;

    // Process includes.
    for include_path_str in &with_includes.include {
        let include_path = base_path
            .parent()
            .unwrap_or(base_path)
            .join(include_path_str);
        load_includes_recursive(&include_path, config, &mut main_specs, &mut visited)?;
    }

    // Main file takes precedence: included specs are only added if their ID
    // isn't already present. This is handled by load_includes_recursive.

    validation::validate_multi(&main_specs)?;

    let mut results = Vec::with_capacity(main_specs.len());
    for spec in &main_specs {
        results.push(compiler::compile(spec)?);
    }

    Ok(results)
}

fn load_includes_recursive(
    path: &Path,
    config: &HashMap<String, serde_json::Value>,
    specs: &mut Vec<schema::WorkflowSpec>,
    visited: &mut HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    let canonical = path.canonicalize().map_err(|e| {
        YamlWorkflowError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Include file not found: {}: {e}", path.display()),
        ))
    })?;

    let canonical_str = canonical.to_string_lossy().to_string();
    if !visited.insert(canonical_str.clone()) {
        return Err(YamlWorkflowError::Validation(format!(
            "Circular include detected: '{}'",
            path.display()
        )));
    }

    let yaml = std::fs::read_to_string(&canonical)?;
    let interpolated = interpolation::interpolate(&yaml, config)?;
    let raw_value: serde_yaml::Value = serde_yaml::from_str(&interpolated)?;
    let merged_value = yaml_merge_keys::merge_keys_serde(raw_value).map_err(|e| {
        YamlWorkflowError::Parse(serde_yaml::Error::custom(format!(
            "merge key resolution failed: {e}"
        )))
    })?;

    let with_includes: YamlWorkflowFileWithIncludes = serde_yaml::from_value(merged_value)?;

    let included_specs = resolve_workflow_specs(with_includes.file)?;

    // Existing IDs in main specs take precedence.
    let existing_ids: HashSet<String> = specs.iter().map(|s| s.id.clone()).collect();
    for spec in included_specs {
        if !existing_ids.contains(&spec.id) {
            specs.push(spec);
        }
    }

    // Recurse into nested includes.
    for nested_include in &with_includes.include {
        let nested_path = canonical
            .parent()
            .unwrap_or(&canonical)
            .join(nested_include);
        load_includes_recursive(&nested_path, config, specs, visited)?;
    }

    Ok(())
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
