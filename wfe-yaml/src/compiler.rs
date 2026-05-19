use std::time::Duration;

use serde::Serialize;
use wfe_core::models::error_behavior::ErrorBehavior;
use wfe_core::models::service::{ReadinessCheck, ReadinessProbe, ServiceDefinition, ServicePort};
use wfe_core::models::workflow_definition::{StepOutcome, WorkflowDefinition, WorkflowStep};
use wfe_core::traits::StepBody;

use crate::error::YamlWorkflowError;
#[cfg(feature = "deno")]
use crate::executors::deno::{DenoConfig, DenoPermissions, DenoStep};
use crate::executors::git_repo::{GitRepoConfig, GitRepoStep};
use crate::executors::shell::{ShellConfig, ShellStep};
#[cfg(feature = "buildkit")]
use wfe_buildkit::{BuildkitConfig, BuildkitStep};
#[cfg(feature = "containerd")]
use wfe_containerd::{ContainerdConfig, ContainerdStep};
use wfe_core::models::condition::{ComparisonOp, FieldComparison, StepCondition};
use wfe_core::primitives::sub_workflow::SubWorkflowStep;
#[cfg(feature = "kubernetes")]
use wfe_kubernetes::{ClusterConfig, KubernetesStep, KubernetesStepConfig};
#[cfg(feature = "rustlang")]
use wfe_rustlang::{CargoCommand, CargoConfig, CargoStep, RustupCommand, RustupConfig, RustupStep};

use crate::schema::{
    WorkflowSpec, YamlCombinator, YamlComparison, YamlCondition, YamlErrorBehavior, YamlStep,
};

/// Configuration for a sub-workflow step.
#[derive(Debug, Clone, Serialize)]
pub struct SubWorkflowConfig {
    /// Workflow id.
    pub workflow_id: String,
    /// Version.
    pub version: u32,
    /// Output keys.
    pub output_keys: Vec<String>,
}

/// Factory type alias for step creation closures.
pub type StepFactory = Box<dyn Fn() -> Box<dyn StepBody> + Send + Sync>;

/// A compiled workflow ready to be registered with the WFE host.
pub struct CompiledWorkflow {
    /// Definition.
    pub definition: WorkflowDefinition,
    /// Step factories.
    pub step_factories: Vec<(String, StepFactory)>,
}

/// Compile a parsed WorkflowSpec into a CompiledWorkflow.
pub fn compile(spec: &WorkflowSpec) -> Result<CompiledWorkflow, YamlWorkflowError> {
    let mut definition = WorkflowDefinition::new(&spec.id, spec.version);
    definition.name = spec.name.clone();
    definition.description = spec.description.clone();
    definition.shared_volume =
        spec.shared_volume
            .as_ref()
            .map(|v| wfe_core::models::SharedVolume {
                mount_path: v.mount_path.clone(),
                size: v.size.clone(),
            });

    if let Some(ref eb) = spec.error_behavior {
        definition.default_error_behavior = map_error_behavior(eb)?;
    }

    let mut factories: Vec<(String, StepFactory)> = Vec::new();
    let mut next_id: usize = 0;

    compile_steps(&spec.steps, &mut definition, &mut factories, &mut next_id)?;

    // Compile services.
    definition.services = compile_services(&spec.services)?;

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

            let mut container =
                WorkflowStep::new(container_id, "wfe_core::primitives::sequence::SequenceStep");
            container.name = Some(yaml_step.name.clone());

            if let Some(ref eb) = yaml_step.error_behavior {
                container.error_behavior = Some(map_error_behavior(eb)?);
            }

            // Compile children.
            let child_ids = compile_steps(parallel_children, definition, factories, next_id)?;
            container.children = child_ids;

            // Compile condition if present.
            if let Some(ref yaml_cond) = yaml_step.when {
                container.when = Some(compile_condition(yaml_cond)?);
            }

            definition.steps.push(container);
            main_step_ids.push(container_id);
        } else {
            // Regular step.
            let step_id = *next_id;
            *next_id += 1;

            let step_type = yaml_step.step_type.as_deref().unwrap_or("shell");

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

            // Compile condition if present.
            if let Some(ref yaml_cond) = yaml_step.when {
                wf_step.when = Some(compile_condition(yaml_cond)?);
            }

            // Handle on_failure: create compensation step.
            if let Some(ref on_failure) = yaml_step.on_failure {
                let comp_id = *next_id;
                *next_id += 1;

                let on_failure_type = on_failure.step_type.as_deref().unwrap_or("shell");
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

                let on_success_type = on_success.step_type.as_deref().unwrap_or("shell");
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

                let ensure_type = ensure.step_type.as_deref().unwrap_or("shell");
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

/// Convert a YAML condition tree into a `StepCondition` tree.
pub fn compile_condition(yaml_cond: &YamlCondition) -> Result<StepCondition, YamlWorkflowError> {
    match yaml_cond {
        YamlCondition::Comparison(cmp) => compile_comparison(cmp.as_ref()),
        YamlCondition::Combinator(combinator) => compile_combinator(combinator),
    }
}

fn compile_combinator(c: &YamlCombinator) -> Result<StepCondition, YamlWorkflowError> {
    // Count how many combinator keys are set to detect ambiguity.
    let mut count = 0;
    if c.all.is_some() {
        count += 1;
    }
    if c.any.is_some() {
        count += 1;
    }
    if c.none.is_some() {
        count += 1;
    }
    if c.one_of.is_some() {
        count += 1;
    }
    if c.not.is_some() {
        count += 1;
    }

    if count == 0 {
        return Err(YamlWorkflowError::Compilation(
            "Condition combinator must have at least one of: all, any, none, one_of, not"
                .to_string(),
        ));
    }
    if count > 1 {
        return Err(YamlWorkflowError::Compilation(
            "Condition combinator must have exactly one of: all, any, none, one_of, not"
                .to_string(),
        ));
    }

    if let Some(ref children) = c.all {
        let compiled: Result<Vec<_>, _> = children.iter().map(compile_condition).collect();
        Ok(StepCondition::All(compiled?))
    } else if let Some(ref children) = c.any {
        let compiled: Result<Vec<_>, _> = children.iter().map(compile_condition).collect();
        Ok(StepCondition::Any(compiled?))
    } else if let Some(ref children) = c.none {
        let compiled: Result<Vec<_>, _> = children.iter().map(compile_condition).collect();
        Ok(StepCondition::None(compiled?))
    } else if let Some(ref children) = c.one_of {
        let compiled: Result<Vec<_>, _> = children.iter().map(compile_condition).collect();
        Ok(StepCondition::OneOf(compiled?))
    } else if let Some(ref inner) = c.not {
        Ok(StepCondition::Not(Box::new(compile_condition(inner)?)))
    } else {
        unreachable!()
    }
}

fn compile_comparison(cmp: &YamlComparison) -> Result<StepCondition, YamlWorkflowError> {
    // Determine which operator is specified. Exactly one must be present.
    let mut ops: Vec<(ComparisonOp, Option<serde_json::Value>)> = Vec::new();

    if let Some(ref v) = cmp.equals {
        ops.push((ComparisonOp::Equals, Some(yaml_value_to_json(v))));
    }
    if let Some(ref v) = cmp.not_equals {
        ops.push((ComparisonOp::NotEquals, Some(yaml_value_to_json(v))));
    }
    if let Some(ref v) = cmp.gt {
        ops.push((ComparisonOp::Gt, Some(yaml_value_to_json(v))));
    }
    if let Some(ref v) = cmp.gte {
        ops.push((ComparisonOp::Gte, Some(yaml_value_to_json(v))));
    }
    if let Some(ref v) = cmp.lt {
        ops.push((ComparisonOp::Lt, Some(yaml_value_to_json(v))));
    }
    if let Some(ref v) = cmp.lte {
        ops.push((ComparisonOp::Lte, Some(yaml_value_to_json(v))));
    }
    if let Some(ref v) = cmp.contains {
        ops.push((ComparisonOp::Contains, Some(yaml_value_to_json(v))));
    }
    if let Some(true) = cmp.is_null {
        ops.push((ComparisonOp::IsNull, None));
    }
    if let Some(true) = cmp.is_not_null {
        ops.push((ComparisonOp::IsNotNull, None));
    }

    if ops.is_empty() {
        return Err(YamlWorkflowError::Compilation(format!(
            "Comparison on field '{}' must specify an operator (equals, gt, etc.)",
            cmp.field
        )));
    }
    if ops.len() > 1 {
        return Err(YamlWorkflowError::Compilation(format!(
            "Comparison on field '{}' must specify exactly one operator, found {}",
            cmp.field,
            ops.len()
        )));
    }

    let (operator, value) = ops.remove(0);
    Ok(StepCondition::Comparison(FieldComparison {
        field: cmp.field.clone(),
        operator,
        value,
    }))
}

/// Convert a serde_yaml::Value to serde_json::Value.
fn yaml_value_to_json(v: &serde_yaml::Value) -> serde_json::Value {
    match v {
        serde_yaml::Value::Null => serde_json::Value::Null,
        serde_yaml::Value::Bool(b) => serde_json::Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::Value::Number(serde_json::Number::from(i))
            } else if let Some(u) = n.as_u64() {
                serde_json::Value::Number(serde_json::Number::from(u))
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                serde_json::Value::Null
            }
        }
        serde_yaml::Value::String(s) => serde_json::Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => {
            serde_json::Value::Array(seq.iter().map(yaml_value_to_json).collect())
        }
        serde_yaml::Value::Mapping(map) => {
            let mut obj = serde_json::Map::new();
            for (k, val) in map {
                if let serde_yaml::Value::String(key) = k {
                    obj.insert(key.clone(), yaml_value_to_json(val));
                }
            }
            serde_json::Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_value_to_json(&tagged.value),
    }
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
                YamlWorkflowError::Compilation(format!("Failed to serialize shell config: {e}"))
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
                YamlWorkflowError::Compilation(format!("Failed to serialize deno config: {e}"))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(DenoStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        #[cfg(feature = "buildkit")]
        "buildkit" => {
            let config = build_buildkit_config(step)?;
            let key = format!("wfe_yaml::buildkit::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!("Failed to serialize buildkit config: {e}"))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(BuildkitStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        #[cfg(feature = "containerd")]
        "containerd" => {
            let config = build_containerd_config(step)?;
            let key = format!("wfe_yaml::containerd::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!(
                    "Failed to serialize containerd config: {e}"
                ))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(ContainerdStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        #[cfg(feature = "kubernetes")]
        "kubernetes" | "k8s" => {
            let config = build_kubernetes_config(step)?;
            let key = format!("wfe_yaml::kubernetes::{}", step.name);
            let value = serde_json::to_value(&config.0).map_err(|e| {
                YamlWorkflowError::Compilation(format!(
                    "Failed to serialize kubernetes config: {e}"
                ))
            })?;
            let step_config = config.0;
            let cluster_config = config.1;
            let factory: StepFactory = Box::new(move || {
                Box::new(KubernetesStep::lazy(
                    step_config.clone(),
                    cluster_config.clone(),
                )) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        #[cfg(feature = "rustlang")]
        "cargo-build" | "cargo-test" | "cargo-check" | "cargo-clippy" | "cargo-fmt"
        | "cargo-doc" | "cargo-publish" | "cargo-audit" | "cargo-deny" | "cargo-nextest"
        | "cargo-llvm-cov" | "cargo-doc-mdx" => {
            let config = build_cargo_config(step, step_type)?;
            let key = format!("wfe_yaml::cargo::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!("Failed to serialize cargo config: {e}"))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(CargoStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        #[cfg(feature = "rustlang")]
        "rust-install" | "rustup-toolchain" | "rustup-component" | "rustup-target" => {
            let config = build_rustup_config(step, step_type)?;
            let key = format!("wfe_yaml::rustup::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!("Failed to serialize rustup config: {e}"))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(RustupStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        "git-repo" => {
            let config = build_git_repo_config(step)?;
            let key = format!("wfe_yaml::git_repo::{}", step.name);
            let value = serde_json::to_value(&config).map_err(|e| {
                YamlWorkflowError::Compilation(format!("Failed to serialize git-repo config: {e}"))
            })?;
            let config_clone = config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(GitRepoStep::new(config_clone.clone())) as Box<dyn StepBody>
            });
            Ok((key, value, factory))
        }
        "workflow" => {
            let config = step.config.as_ref().ok_or_else(|| {
                YamlWorkflowError::Compilation(format!(
                    "Workflow step '{}' is missing 'config' section",
                    step.name
                ))
            })?;
            let child_workflow_id = config.child_workflow.as_ref().ok_or_else(|| {
                YamlWorkflowError::Compilation(format!(
                    "Workflow step '{}' must have 'config.workflow'",
                    step.name
                ))
            })?;
            let child_version = config.child_version.unwrap_or(1);

            let sub_config = SubWorkflowConfig {
                workflow_id: child_workflow_id.clone(),
                version: child_version,
                output_keys: step.outputs.iter().map(|o| o.name.clone()).collect(),
            };

            let key = format!("wfe_yaml::workflow::{}", step.name);
            let value = serde_json::to_value(&sub_config).map_err(|e| {
                YamlWorkflowError::Compilation(format!("Failed to serialize workflow config: {e}"))
            })?;
            let config_clone = sub_config.clone();
            let factory: StepFactory = Box::new(move || {
                Box::new(SubWorkflowStep {
                    workflow_id: config_clone.workflow_id.clone(),
                    version: config_clone.version,
                    output_keys: config_clone.output_keys.clone(),
                    inputs: serde_json::Value::Null,
                    input_schema: None,
                    output_schema: None,
                }) as Box<dyn StepBody>
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
        YamlWorkflowError::Compilation(format!("Step '{}' is missing 'config' section", step.name))
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
        inputs: Some(config.inputs.clone()).filter(|m| !m.is_empty()),
    })
}

fn build_git_repo_config(step: &YamlStep) -> Result<GitRepoConfig, YamlWorkflowError> {
    let config = step.config.as_ref().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "git-repo step '{}' is missing 'config' section",
            step.name
        ))
    })?;

    let url = config
        .run
        .clone()
        .or_else(|| config.script.clone())
        .ok_or_else(|| {
            YamlWorkflowError::Compilation(format!(
                "git-repo step '{}' must have 'config.run' (the remote URL)",
                step.name
            ))
        })?;

    Ok(GitRepoConfig {
        url,
        branch: config.branch.clone(),
        commit: config.commit.clone(),
        depth: config.depth,
        path: config
            .working_dir
            .clone()
            .unwrap_or_else(|| "/workspace/repo".to_string()),
        output: step.name.clone(),
        input: config.input.clone(),
    })
}

#[cfg(feature = "rustlang")]
fn build_cargo_config(step: &YamlStep, step_type: &str) -> Result<CargoConfig, YamlWorkflowError> {
    let command = match step_type {
        "cargo-build" => CargoCommand::Build,
        "cargo-test" => CargoCommand::Test,
        "cargo-check" => CargoCommand::Check,
        "cargo-clippy" => CargoCommand::Clippy,
        "cargo-fmt" => CargoCommand::Fmt,
        "cargo-doc" => CargoCommand::Doc,
        "cargo-publish" => CargoCommand::Publish,
        "cargo-audit" => CargoCommand::Audit,
        "cargo-deny" => CargoCommand::Deny,
        "cargo-nextest" => CargoCommand::Nextest,
        "cargo-llvm-cov" => CargoCommand::LlvmCov,
        "cargo-doc-mdx" => CargoCommand::DocMdx,
        _ => {
            return Err(YamlWorkflowError::Compilation(format!(
                "Unknown cargo step type: '{step_type}'"
            )));
        }
    };

    let config = step.config.as_ref();
    let timeout_ms = config
        .and_then(|c| c.timeout.as_ref())
        .and_then(|t| parse_duration_ms(t));

    Ok(CargoConfig {
        command,
        toolchain: config.and_then(|c| c.toolchain.clone()),
        package: config.and_then(|c| c.package.clone()),
        features: config.map(|c| c.features.clone()).unwrap_or_default(),
        all_features: config.and_then(|c| c.all_features).unwrap_or(false),
        no_default_features: config.and_then(|c| c.no_default_features).unwrap_or(false),
        release: config.and_then(|c| c.release).unwrap_or(false),
        target: config.and_then(|c| c.target.clone()),
        profile: config.and_then(|c| c.profile.clone()),
        extra_args: config.map(|c| c.extra_args.clone()).unwrap_or_default(),
        env: config.map(|c| c.env.clone()).unwrap_or_default(),
        working_dir: config.and_then(|c| c.working_dir.clone()),
        timeout_ms,
        output_dir: config.and_then(|c| c.output_dir.clone()),
    })
}

#[cfg(feature = "rustlang")]
fn build_rustup_config(
    step: &YamlStep,
    step_type: &str,
) -> Result<RustupConfig, YamlWorkflowError> {
    let command = match step_type {
        "rust-install" => RustupCommand::Install,
        "rustup-toolchain" => RustupCommand::ToolchainInstall,
        "rustup-component" => RustupCommand::ComponentAdd,
        "rustup-target" => RustupCommand::TargetAdd,
        _ => {
            return Err(YamlWorkflowError::Compilation(format!(
                "Unknown rustup step type: '{step_type}'"
            )));
        }
    };

    let config = step.config.as_ref();
    let timeout_ms = config
        .and_then(|c| c.timeout.as_ref())
        .and_then(|t| parse_duration_ms(t));

    Ok(RustupConfig {
        command,
        toolchain: config.and_then(|c| c.toolchain.clone()),
        components: config.map(|c| c.components.clone()).unwrap_or_default(),
        targets: config.map(|c| c.targets.clone()).unwrap_or_default(),
        profile: config.and_then(|c| c.profile.clone()),
        default_toolchain: config.and_then(|c| c.default_toolchain.clone()),
        extra_args: config.map(|c| c.extra_args.clone()).unwrap_or_default(),
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

#[cfg(feature = "buildkit")]
fn build_buildkit_config(step: &YamlStep) -> Result<BuildkitConfig, YamlWorkflowError> {
    let config = step.config.as_ref().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "BuildKit step '{}' is missing 'config' section",
            step.name
        ))
    })?;

    let dockerfile = config.dockerfile.clone().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "BuildKit step '{}' must have 'config.dockerfile'",
            step.name
        ))
    })?;

    let context = config.context.clone().unwrap_or_else(|| {
        // When an artifact input overrides the context at runtime,
        // a placeholder default is sufficient at compile time.
        if config.input.is_some() {
            ".".to_string()
        } else {
            String::new()
        }
    });

    if context.is_empty() {
        return Err(YamlWorkflowError::Compilation(format!(
            "BuildKit step '{}' must have 'config.context'",
            step.name
        )));
    }

    let timeout_ms = config.timeout.as_ref().and_then(|t| parse_duration_ms(t));

    let tls = config
        .tls
        .as_ref()
        .map(|t| wfe_buildkit::TlsConfig {
            ca: t.ca.clone(),
            cert: t.cert.clone(),
            key: t.key.clone(),
        })
        .unwrap_or_default();

    let registry_auth = config
        .registry_auth
        .as_ref()
        .map(|ra| {
            ra.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        wfe_buildkit::RegistryAuth {
                            username: v.username.clone(),
                            password: v.password.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(BuildkitConfig {
        dockerfile,
        context,
        target: config.target.clone(),
        tags: config.tags.clone(),
        build_args: config.build_args.clone(),
        cache_from: config.cache_from.clone(),
        cache_to: config.cache_to.clone(),
        push: config.push.unwrap_or(false),
        output_type: config.output_type.clone(),
        output_dest: config.output_dest.clone(),
        inputs: Some(config.inputs.clone()).filter(|m| !m.is_empty()),
        input: config.input.clone(),
        buildkit_addr: config
            .buildkit_addr
            .clone()
            .unwrap_or_else(|| "unix:///run/buildkit/buildkitd.sock".to_string()),
        tls,
        registry_auth,
        timeout_ms,
    })
}

#[cfg(feature = "containerd")]
fn build_containerd_config(step: &YamlStep) -> Result<ContainerdConfig, YamlWorkflowError> {
    let config = step.config.as_ref().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "Containerd step '{}' is missing 'config' section",
            step.name
        ))
    })?;

    let image = config.image.clone().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "Containerd step '{}' must have 'config.image'",
            step.name
        ))
    })?;

    let timeout_ms = config.timeout.as_ref().and_then(|t| parse_duration_ms(t));

    let tls = config
        .tls
        .as_ref()
        .map(|t| wfe_containerd::TlsConfig {
            ca: t.ca.clone(),
            cert: t.cert.clone(),
            key: t.key.clone(),
        })
        .unwrap_or_default();

    let registry_auth = config
        .registry_auth
        .as_ref()
        .map(|ra| {
            ra.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        wfe_containerd::RegistryAuth {
                            username: v.username.clone(),
                            password: v.password.clone(),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default();

    let volumes = config
        .volumes
        .iter()
        .map(|v| wfe_containerd::VolumeMountConfig {
            source: v.source.clone(),
            target: v.target.clone(),
            readonly: v.readonly,
        })
        .collect();

    Ok(ContainerdConfig {
        image,
        command: config.command.clone(),
        run: config.run.clone(),
        env: config.env.clone(),
        volumes,
        working_dir: config.working_dir.clone(),
        user: config
            .user
            .clone()
            .unwrap_or_else(|| "65534:65534".to_string()),
        network: config.network.clone().unwrap_or_else(|| "none".to_string()),
        memory: config.memory.clone(),
        cpu: config.cpu.clone(),
        pull: config
            .pull
            .clone()
            .unwrap_or_else(|| "if-not-present".to_string()),
        containerd_addr: config
            .containerd_addr
            .clone()
            .unwrap_or_else(|| "/run/containerd/containerd.sock".to_string()),
        cli: config.cli.clone().unwrap_or_else(|| "nerdctl".to_string()),
        tls,
        registry_auth,
        inputs: Some(config.inputs.clone()).filter(|m| !m.is_empty()),
        timeout_ms,
    })
}

#[cfg(feature = "kubernetes")]
fn build_kubernetes_config(
    step: &YamlStep,
) -> Result<(KubernetesStepConfig, ClusterConfig), YamlWorkflowError> {
    let config = step.config.as_ref().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "Kubernetes step '{}' is missing 'config' section",
            step.name
        ))
    })?;

    let image = config.image.clone().ok_or_else(|| {
        YamlWorkflowError::Compilation(format!(
            "Kubernetes step '{}' must have 'config.image'",
            step.name
        ))
    })?;

    let timeout_ms = config.timeout.as_ref().and_then(|t| parse_duration_ms(t));

    let step_config = KubernetesStepConfig {
        image,
        command: config.command.clone(),
        run: config.run.clone(),
        shell: config.shell.clone(),
        env: config.env.clone(),
        working_dir: config.working_dir.clone(),
        memory: config.memory.clone(),
        cpu: config.cpu.clone(),
        timeout_ms,
        pull_policy: config.pull_policy.clone(),
        namespace: config.namespace.clone(),
    };

    let cluster_config = ClusterConfig {
        kubeconfig: config.kubeconfig.clone(),
        ..Default::default()
    };

    Ok((step_config, cluster_config))
}

fn compile_services(
    yaml_services: &std::collections::HashMap<String, crate::schema::YamlService>,
) -> Result<Vec<ServiceDefinition>, YamlWorkflowError> {
    let mut services = Vec::new();

    for (name, yaml_svc) in yaml_services {
        let readiness = yaml_svc.readiness.as_ref().map(|r| {
            let check = if let Some(ref exec_cmd) = r.exec {
                ReadinessCheck::Exec(exec_cmd.clone())
            } else if let Some(tcp_port) = r.tcp {
                ReadinessCheck::TcpSocket(tcp_port)
            } else if let Some(ref http) = r.http {
                ReadinessCheck::HttpGet {
                    port: http.port,
                    path: http.path.clone(),
                }
            } else {
                // Default: TCP check on first port.
                ReadinessCheck::TcpSocket(yaml_svc.ports.first().copied().unwrap_or(0))
            };

            let interval_ms = r
                .interval
                .as_ref()
                .and_then(|s| parse_duration_ms(s))
                .unwrap_or(5000);
            let timeout_ms = r
                .timeout
                .as_ref()
                .and_then(|s| parse_duration_ms(s))
                .unwrap_or(60000);

            ReadinessProbe {
                check,
                interval_ms,
                timeout_ms,
                retries: r.retries.unwrap_or(12),
            }
        });

        services.push(ServiceDefinition {
            name: name.clone(),
            image: yaml_svc.image.clone(),
            ports: yaml_svc
                .ports
                .iter()
                .map(|&p| ServicePort::tcp(p))
                .collect(),
            env: yaml_svc.env.clone(),
            readiness,
            command: yaml_svc.command.clone().unwrap_or_default(),
            args: yaml_svc.args.clone().unwrap_or_default(),
            memory: yaml_svc.memory.clone(),
            cpu: yaml_svc.cpu.clone(),
        });
    }

    Ok(services)
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
