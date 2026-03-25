use std::collections::HashSet;

use crate::error::YamlWorkflowError;
use crate::schema::{WorkflowSpec, YamlStep};

/// Validate a parsed workflow spec.
pub fn validate(spec: &WorkflowSpec) -> Result<(), YamlWorkflowError> {
    if spec.steps.is_empty() {
        return Err(YamlWorkflowError::Validation(
            "Workflow must have at least one step".to_string(),
        ));
    }

    let mut seen_names = HashSet::new();
    validate_steps(&spec.steps, &mut seen_names)?;

    // Validate workflow-level error behavior.
    if let Some(ref eb) = spec.error_behavior {
        validate_error_behavior_type(&eb.behavior_type)?;
    }

    Ok(())
}

fn validate_steps(
    steps: &[YamlStep],
    seen_names: &mut HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    for step in steps {
        // Check for duplicate names.
        if !seen_names.insert(step.name.clone()) {
            return Err(YamlWorkflowError::Validation(format!(
                "Duplicate step name: '{}'",
                step.name
            )));
        }

        // A step must have either (type + config) or parallel, but not both.
        let has_type = step.step_type.is_some();
        let has_parallel = step.parallel.is_some();

        if !has_type && !has_parallel {
            return Err(YamlWorkflowError::Validation(format!(
                "Step '{}' must have either 'type' + 'config' or 'parallel'",
                step.name
            )));
        }

        if has_type && has_parallel {
            return Err(YamlWorkflowError::Validation(format!(
                "Step '{}' cannot have both 'type' and 'parallel'",
                step.name
            )));
        }

        // Shell steps must have config.run or config.file.
        if let Some(ref step_type) = step.step_type
            && step_type == "shell"
        {
            let config = step.config.as_ref().ok_or_else(|| {
                YamlWorkflowError::Validation(format!(
                    "Shell step '{}' must have a 'config' section",
                    step.name
                ))
            })?;
            if config.run.is_none() && config.file.is_none() {
                return Err(YamlWorkflowError::Validation(format!(
                    "Shell step '{}' must have 'config.run' or 'config.file'",
                    step.name
                )));
            }
        }

        // Validate step-level error behavior.
        if let Some(ref eb) = step.error_behavior {
            validate_error_behavior_type(&eb.behavior_type)?;
        }

        // Validate parallel children.
        if let Some(ref children) = step.parallel {
            validate_steps(children, seen_names)?;
        }

        // Validate hook steps.
        if let Some(ref hook) = step.on_success {
            validate_steps(std::slice::from_ref(hook.as_ref()), seen_names)?;
        }
        if let Some(ref hook) = step.on_failure {
            validate_steps(std::slice::from_ref(hook.as_ref()), seen_names)?;
        }
        if let Some(ref hook) = step.ensure {
            validate_steps(std::slice::from_ref(hook.as_ref()), seen_names)?;
        }
    }
    Ok(())
}

fn validate_error_behavior_type(behavior_type: &str) -> Result<(), YamlWorkflowError> {
    match behavior_type {
        "retry" | "suspend" | "terminate" | "compensate" => Ok(()),
        other => Err(YamlWorkflowError::Validation(format!(
            "Invalid error behavior type: '{}'. Must be retry, suspend, terminate, or compensate",
            other
        ))),
    }
}
