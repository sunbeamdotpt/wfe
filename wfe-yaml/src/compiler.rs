use std::time::Duration;

use wfe_core::models::error_behavior::ErrorBehavior;
use wfe_core::models::workflow_definition::{StepOutcome, WorkflowDefinition, WorkflowStep};
use wfe_core::traits::StepBody;

use crate::error::YamlWorkflowError;
use crate::executors::shell::{ShellConfig, ShellStep};
#[cfg(feature = "deno")]
use crate::executors::deno::{DenoConfig, DenoPermissions, DenoStep};
use crate::schema::{WorkflowSpec, YamlErrorBehavior, YamlStep};

/// Factory type alias for step creation closures.
pub type StepFactory = Box<dyn Fn() -> Box<dyn StepBody> + Send + Sync>;

/// A compiled workflow ready to be registered with the WFE host.
pub struct CompiledWorkflow {
    pub definition: WorkflowDefinition,
    pub step_factories: Vec<(String, StepFactory)>,
}

/// Compile a parsed WorkflowSpec into a CompiledWorkflow.
pub fn compile(spec: &WorkflowSpec) -> Result<CompiledWorkflow, YamlWorkflowError> {
    let mut definition = WorkflowDefinition::new(&spec.id, spec.version);
    definition.description = spec.description.clone();

    if let Some(ref eb) = spec.error_behavior {
        definition.default_error_behavior = map_error_behavior(eb)?;
    }

    let mut factories: Vec<(String, StepFactory)> = Vec::new();
    let mut next_id: usize = 0;

    compile_steps(&spec.steps, &mut definition, &mut factories, &mut next_id)?;

    Ok(CompiledWorkflow {
        definition,
        step_factories: factories,
    })
}

fn compile_steps(
    yaml_steps: &[YamlStep],
    definition: &mut WorkflowDefinition,
    factories: &mut Vec<(String, StepFactory)>,
    next_id: &mut usize,
) -> Result<Vec<usize>, YamlWorkflowError> {
    let mut main_step_ids = Vec::new();

    for yaml_step in yaml_steps {
        if let Some(ref parallel_children) = yaml_step.parallel {
            // Create a Sequence container step for the parallel block.
            let container_id = *next_id;
            *next_id += 1;

            let mut container = WorkflowStep::new(
                container_id,
                "wfe_core::primitives::sequence::SequenceStep",
            );
            container.name = Some(yaml_step.name.clone());

            if let Some(ref eb) = yaml_step.error_behavior {
                container.error_behavior = Some(map_error_behavior(eb)?);
            }

            // Compile children.
            let child_ids =
                compile_steps(parallel_children, definition, factories, next_id)?;
            container.children = child_ids;

            definition.steps.push(container);
            main_step_ids.push(container_id);
        } else {
            // Regular step.
            let step_id = *next_id;
            *next_id += 1;

            let step_type = yaml_step
                .step_type
                .as_deref()
                .unwrap_or("shell");

            let (step_type_key, step_config_value, factory): (
                String,
                serde_json::Value,
                StepFactory,
            ) = build_step_config_and_factory(yaml_step, step_type)?;

            let mut wf_step = WorkflowStep::new(step_id, &step_type_key);
            wf_step.name = Some(yaml_step.name.clone());
            wf_step.step_config = Some(step_config_value);

            if let Some(ref eb) = yaml_step.error_behavior {
                wf_step.error_behavior = Some(map_error_behavior(eb)?);
            }

            // Handle on_failure: create compensation step.
            if let Some(ref on_failure) = yaml_step.on_failure {
                let comp_id = *next_id;
                *next_id += 1;

                let on_failure_type = on_failure
                    .step_type
                    .as_deref()
                    .unwrap_or("shell");
                let (comp_key, comp_config_value, comp_factory) =
                    build_step_config_and_factory(on_failure, on_failure_type)?;

                let mut comp_step = WorkflowStep::new(comp_id, &comp_key);
                comp_step.name = Some(on_failure.name.clone());
                comp_step.step_config = Some(comp_config_value);

                wf_step.compensation_step_id = Some(comp_id);
                wf_step.error_behavior = Some(ErrorBehavior::Compensate);

                definition.steps.push(comp_step);
                factories.push((comp_key, comp_factory));
            }

            // Handle on_success: insert between this step and the next.
            if let Some(ref on_success) = yaml_step.on_success {
                let success_id = *next_id;
                *next_id += 1;

                let on_success_type = on_success
                    .step_type
                    .as_deref()
                    .unwrap_or("shell");
                let (success_key, success_config_value, success_factory) =
                    build_step_config_and_factory(on_success, on_success_type)?;

                let mut success_step = WorkflowStep::new(success_id, &success_key);
                success_step.name = Some(on_success.name.clone());
                success_step.step_config = Some(success_config_value);

                // Wire main step -> on_success step.
                wf_step.outcomes.push(StepOutcome {
                    next_step: success_id,
                    label: Some("success".to_string()),
                    value: None,
                });

                definition.steps.push(success_step);
                factories.push((success_key, success_factory));
            }

            // Handle ensure: create an ensure step wired after both paths.
            if let Some(ref ensure) = yaml_step.ensure {
                let ensure_id = *next_id;
                *next_id += 1;

                let ensure_type = ensure
                    .step_type
                    .as_deref()
                    .unwrap_or("shell");
                let (ensure_key, ensure_config_value, ensure_factory) =
                    build_step_config_and_factory(ensure, ensure_type)?;

                let mut ensure_step = WorkflowStep::new(ensure_id, &ensure_key);
                ensure_step.name = Some(ensure.name.clone());
                ensure_step.step_config = Some(ensure_config_value);

                // Wire main step -> ensure (if no on_success already).
                if yaml_step.on_success.is_none() {
                    wf_step.outcomes.push(StepOutcome {
                        next_step: ensure_id,
                        label: Some("ensure".to_string()),
                        value: None,
                    });
                }

                definition.steps.push(ensure_step);
                factories.push((ensure_key, ensure_factory));
            }

            definition.steps.push(wf_step);

            // Register factory for main step.
            factories.push((step_type_key, factory));

            main_step_ids.push(step_id);
        }
    }

    // Wire sequential outcomes between main steps (step N -> step N+1).
    for i in 0..main_step_ids.len().saturating_sub(1) {
        let current_id = main_step_ids[i];
        let next_step_id = main_step_ids[i + 1];

        if let Some(step) = definition.steps.iter_mut().find(|s| s.id == current_id) {
            if step.outcomes.is_empty() {
                step.outcomes.push(StepOutcome {
                    next_step: next_step_id,
                    label: None,
                    value: None,
                });
            } else {
                // Wire the last hook step to the next main step.
                let last_outcome_step = step.outcomes.last().unwrap().next_step;
                if let Some(hook_step) = definition
                    .steps
                    .iter_mut()
                    .find(|s| s.id == last_outcome_step)
                    && hook_step.outcomes.is_empty()
                {
                    hook_step.outcomes.push(StepOutcome {
                        next_step: next_step_id,
                        label: None,
                        value: None,
                    });
                }
            }
        }
    }

    Ok(main_step_ids)
}

fn build_step_config_and_factory(
    step: &YamlStep,
    step_type: &str,
) -> Result<(String, serde_json::Value, StepFactory), YamlWorkflowError> {
    match step_type {
        "shell" => {
            let config = build_shell_config(step)?;
            let key = format!("wfe_yaml::shell::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!(
                    "Failed to serialize shell config: {e}"
                ))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(ShellStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        #[cfg(feature = "deno")]
        "deno" => {
            let config = build_deno_config(step)?;
            let key = format!("wfe_yaml::deno::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!(
                    "Failed to serialize deno config: {e}"
                ))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(DenoStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        other => Err(YamlWorkflowError::Compilation(format!(
            "Unknown step type: '{other}'"
        ))),
    }
}

#[cfg(feature = "deno")]
fn build_deno_config(step: &YamlStep) -> Result<DenoConfig, YamlWorkflowError> {
    let config = step.config.as_ref().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "Deno step '{}' is missing 'config' section",
            step.name
        ))
    })?;

    let script = config.script.clone();
    let file = config.file.clone();

    if script.is_none() && file.is_none() {
        return Err(YamlWorkflowError::Compilation(format!(
            "Deno step '{}' must have 'script' or 'file' in config",
            step.name
        )));
    }

    let timeout_ms = config.timeout.as_ref().and_then(|t| parse_duration_ms(t));

    let permissions = config
        .permissions
        .as_ref()
        .map(|p| DenoPermissions {
            net: p.net.clone(),
            read: p.read.clone(),
            write: p.write.clone(),
            env: p.env.clone(),
            run: p.run,
            dynamic_import: p.dynamic_import,
        })
        .unwrap_or_default();

    Ok(DenoConfig {
        script,
        file,
        permissions,
        modules: config.modules.clone(),
        env: config.env.clone(),
        timeout_ms,
    })
}

fn build_shell_config(step: &YamlStep) -> Result<ShellConfig, YamlWorkflowError> {
    let config = step.config.as_ref().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "Step '{}' is missing 'config' section",
            step.name
        ))
    })?;

    let run = config
        .run
        .clone()
        .or_else(|| config.file.as_ref().map(|f| format!("sh {f}")))
        .or_else(|| config.script.clone())
        .ok_or_else(|| {
            YamlWorkflowError::Compilation(format!(
                "Step '{}' must have 'run', 'file', or 'script' in config",
                step.name
            ))
        })?;

    let shell = config.shell.clone().unwrap_or_else(|| "sh".to_string());
    let timeout_ms = config.timeout.as_ref().and_then(|t| parse_duration_ms(t));

    Ok(ShellConfig {
        run,
        shell,
        env: config.env.clone(),
        working_dir: config.working_dir.clone(),
        timeout_ms,
    })
}

fn parse_duration_ms(s: &str) -> Option<u64> {
    let s = s.trim();
    // Check "ms" before "s" since strip_suffix('s') would also match "500ms"
    if let Some(ms) = s.strip_suffix("ms") {
        ms.trim().parse::<u64>().ok()
    } else if let Some(secs) = s.strip_suffix('s') {
        secs.trim().parse::<u64>().ok().map(|v| v * 1000)
    } else if let Some(mins) = s.strip_suffix('m') {
        mins.trim().parse::<u64>().ok().map(|v| v * 60 * 1000)
    } else {
        s.parse::<u64>().ok()
    }
}

fn map_error_behavior(eb: &YamlErrorBehavior) -> Result<ErrorBehavior, YamlWorkflowError> {
    match eb.behavior_type.as_str() {
        "retry" => {
            let interval = eb
                .interval
                .as_ref()
                .and_then(|i| parse_duration_ms(i))
                .map(Duration::from_millis)
                .unwrap_or(Duration::from_secs(60));
            let max_retries = eb.max_retries.unwrap_or(3);
            Ok(ErrorBehavior::Retry {
                interval,
                max_retries,
            })
        }
        "suspend" => Ok(ErrorBehavior::Suspend),
        "terminate" => Ok(ErrorBehavior::Terminate),
        "compensate" => Ok(ErrorBehavior::Compensate),
        other => Err(YamlWorkflowError::Compilation(format!(
            "Unknown error behavior type: '{other}'"
        ))),
    }
}
