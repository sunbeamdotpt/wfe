use std::collections::{HashMap, HashSet};

use crate::error::YamlWorkflowError;
use crate::schema::{WorkflowSpec, YamlCombinator, YamlComparison, YamlCondition, YamlStep};
use crate::types::{SchemaType, parse_type_string};

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

    // Collect known outputs (from step output data refs).
    let known_outputs: HashSet<String> = collect_step_outputs(&spec.steps);

    // Validate condition fields and types on all steps.
    validate_step_conditions(&spec.steps, spec, &known_outputs)?;

    // Detect unused declared outputs.
    detect_unused_outputs(spec, &known_outputs)?;

    Ok(())
}

/// Validate multiple workflow specs from a multi-workflow file.
/// Checks cross-workflow references and cycles in addition to per-workflow validation.
pub fn validate_multi(specs: &[WorkflowSpec]) -> Result<(), YamlWorkflowError> {
    // Validate each workflow individually.
    for spec in specs {
        validate(spec)?;
    }

    // Check for duplicate workflow IDs.
    let mut seen_ids = HashSet::new();
    for spec in specs {
        if !seen_ids.insert(&spec.id) {
            return Err(YamlWorkflowError::Validation(format!(
                "Duplicate workflow ID: '{}'",
                spec.id
            )));
        }
    }

    // Validate cross-workflow references and detect cycles.
    validate_workflow_references(specs)?;

    Ok(())
}

/// Validate that workflow step references point to known workflows
/// and detect circular dependencies.
fn validate_workflow_references(specs: &[WorkflowSpec]) -> Result<(), YamlWorkflowError> {
    let known_ids: HashSet<&str> = specs.iter().map(|s| s.id.as_str()).collect();

    // Build a dependency graph: workflow_id -> set of referenced workflow_ids.
    let mut deps: HashMap<&str, HashSet<&str>> = HashMap::new();

    for spec in specs {
        let mut spec_deps = HashSet::new();
        collect_workflow_refs(&spec.steps, &mut spec_deps);
        deps.insert(spec.id.as_str(), spec_deps);
    }

    // Detect cycles using DFS with coloring.
    detect_cycles(&known_ids, &deps)?;

    Ok(())
}

/// Collect all workflow IDs referenced by `type: workflow` steps.
fn collect_workflow_refs<'a>(steps: &'a [YamlStep], refs: &mut HashSet<&'a str>) {
    for step in steps {
        if step.step_type.as_deref() == Some("workflow")
            && let Some(ref config) = step.config
            && let Some(ref wf_id) = config.child_workflow
        {
            refs.insert(wf_id.as_str());
        }
        if let Some(ref children) = step.parallel {
            collect_workflow_refs(children, refs);
        }
        if let Some(ref hook) = step.on_success {
            collect_workflow_refs(std::slice::from_ref(hook.as_ref()), refs);
        }
        if let Some(ref hook) = step.on_failure {
            collect_workflow_refs(std::slice::from_ref(hook.as_ref()), refs);
        }
        if let Some(ref hook) = step.ensure {
            collect_workflow_refs(std::slice::from_ref(hook.as_ref()), refs);
        }
    }
}

/// Detect circular references in the workflow dependency graph.
fn detect_cycles(
    known_ids: &HashSet<&str>,
    deps: &HashMap<&str, HashSet<&str>>,
) -> Result<(), YamlWorkflowError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut colors: HashMap<&str, Color> = known_ids.iter().map(|id| (*id, Color::White)).collect();

    fn dfs<'a>(
        node: &'a str,
        deps: &HashMap<&str, HashSet<&'a str>>,
        colors: &mut HashMap<&'a str, Color>,
        path: &mut Vec<&'a str>,
    ) -> Result<(), YamlWorkflowError> {
        colors.insert(node, Color::Gray);
        path.push(node);

        if let Some(neighbors) = deps.get(node) {
            for &neighbor in neighbors {
                match colors.get(neighbor) {
                    Some(Color::Gray) => {
                        // Found a cycle. Build the cycle path for the error message.
                        let cycle_start = path.iter().position(|&n| n == neighbor).unwrap();
                        let cycle: Vec<&str> = path[cycle_start..].to_vec();
                        return Err(YamlWorkflowError::Validation(format!(
                            "Circular workflow reference detected: {} -> {}",
                            cycle.join(" -> "),
                            neighbor
                        )));
                    }
                    Some(Color::White) | None => {
                        // Only recurse into nodes that are in our known set.
                        if colors.contains_key(neighbor) {
                            dfs(neighbor, deps, colors, path)?;
                        }
                    }
                    Some(Color::Black) => {
                        // Already fully processed, skip.
                    }
                }
            }
        }

        path.pop();
        colors.insert(node, Color::Black);
        Ok(())
    }

    let nodes: Vec<&str> = known_ids.iter().copied().collect();
    for node in nodes {
        if colors.get(node) == Some(&Color::White) {
            let mut path = Vec::new();
            dfs(node, deps, &mut colors, &mut path)?;
        }
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

        // Deno steps must have config with script or file.
        if let Some(ref step_type) = step.step_type
            && step_type == "deno"
        {
            let config = step.config.as_ref().ok_or_else(|| {
                YamlWorkflowError::Validation(format!(
                    "Deno step '{}' must have a 'config' section",
                    step.name
                ))
            })?;
            if config.script.is_none() && config.file.is_none() {
                return Err(YamlWorkflowError::Validation(format!(
                    "Deno step '{}' must have 'config.script' or 'config.file'",
                    step.name
                )));
            }
        }

        // BuildKit steps must have config with dockerfile and context.
        if let Some(ref step_type) = step.step_type
            && step_type == "buildkit"
        {
            let config = step.config.as_ref().ok_or_else(|| {
                YamlWorkflowError::Validation(format!(
                    "BuildKit step '{}' must have a 'config' section",
                    step.name
                ))
            })?;
            if config.dockerfile.is_none() {
                return Err(YamlWorkflowError::Validation(format!(
                    "BuildKit step '{}' must have 'config.dockerfile'",
                    step.name
                )));
            }
            if config.context.is_none() && config.input.is_none() {
                return Err(YamlWorkflowError::Validation(format!(
                    "BuildKit step '{}' must have 'config.context' (or 'config.input' for artifact override)",
                    step.name
                )));
            }
            if config.push.unwrap_or(false) && config.tags.is_empty() {
                return Err(YamlWorkflowError::Validation(format!(
                    "BuildKit step '{}' has push=true but no tags specified",
                    step.name
                )));
            }
        }

        // Containerd steps must have config with image and exactly one of run or command.
        if let Some(ref step_type) = step.step_type
            && step_type == "containerd"
        {
            let config = step.config.as_ref().ok_or_else(|| {
                YamlWorkflowError::Validation(format!(
                    "Containerd step '{}' must have a 'config' section",
                    step.name
                ))
            })?;
            if config.image.is_none() {
                return Err(YamlWorkflowError::Validation(format!(
                    "Containerd step '{}' must have 'config.image'",
                    step.name
                )));
            }
            let has_run = config.run.is_some();
            let has_command = config.command.is_some();
            if !has_run && !has_command {
                return Err(YamlWorkflowError::Validation(format!(
                    "Containerd step '{}' must have 'config.run' or 'config.command'",
                    step.name
                )));
            }
            if has_run && has_command {
                return Err(YamlWorkflowError::Validation(format!(
                    "Containerd step '{}' cannot have both 'config.run' and 'config.command'",
                    step.name
                )));
            }
            if let Some(ref network) = config.network {
                match network.as_str() {
                    "none" | "host" | "bridge" => {}
                    other => {
                        return Err(YamlWorkflowError::Validation(format!(
                            "Containerd step '{}' has invalid network '{}'. Must be none, host, or bridge",
                            step.name, other
                        )));
                    }
                }
            }
            if let Some(ref pull) = config.pull {
                match pull.as_str() {
                    "always" | "if-not-present" | "never" => {}
                    other => {
                        return Err(YamlWorkflowError::Validation(format!(
                            "Containerd step '{}' has invalid pull policy '{}'. Must be always, if-not-present, or never",
                            step.name, other
                        )));
                    }
                }
            }
        }

        // Workflow steps must have config.workflow.
        if let Some(ref step_type) = step.step_type
            && step_type == "workflow"
        {
            let config = step.config.as_ref().ok_or_else(|| {
                YamlWorkflowError::Validation(format!(
                    "Workflow step '{}' must have a 'config' section",
                    step.name
                ))
            })?;
            if config.child_workflow.is_none() {
                return Err(YamlWorkflowError::Validation(format!(
                    "Workflow step '{}' must have 'config.workflow'",
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

// --- Condition validation ---

/// Collect all output field names produced by steps (via their `outputs:` list).
fn collect_step_outputs(steps: &[YamlStep]) -> HashSet<String> {
    let mut outputs = HashSet::new();
    for step in steps {
        for out in &step.outputs {
            outputs.insert(out.name.clone());
        }
        if let Some(ref children) = step.parallel {
            outputs.extend(collect_step_outputs(children));
        }
        if let Some(ref hook) = step.on_success {
            outputs.extend(collect_step_outputs(std::slice::from_ref(hook.as_ref())));
        }
        if let Some(ref hook) = step.on_failure {
            outputs.extend(collect_step_outputs(std::slice::from_ref(hook.as_ref())));
        }
        if let Some(ref hook) = step.ensure {
            outputs.extend(collect_step_outputs(std::slice::from_ref(hook.as_ref())));
        }
    }
    outputs
}

/// Walk all steps and validate their `when` conditions.
fn validate_step_conditions(
    steps: &[YamlStep],
    spec: &WorkflowSpec,
    known_outputs: &HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    for step in steps {
        if let Some(ref cond) = step.when {
            validate_condition_fields(cond, spec, known_outputs)?;
            validate_condition_types(cond, spec)?;
        }
        if let Some(ref children) = step.parallel {
            validate_step_conditions(children, spec, known_outputs)?;
        }
        if let Some(ref hook) = step.on_success {
            validate_step_conditions(std::slice::from_ref(hook.as_ref()), spec, known_outputs)?;
        }
        if let Some(ref hook) = step.on_failure {
            validate_step_conditions(std::slice::from_ref(hook.as_ref()), spec, known_outputs)?;
        }
        if let Some(ref hook) = step.ensure {
            validate_step_conditions(std::slice::from_ref(hook.as_ref()), spec, known_outputs)?;
        }
    }
    Ok(())
}

/// Validate that all field paths in a condition tree resolve to known schema fields.
pub fn validate_condition_fields(
    condition: &YamlCondition,
    spec: &WorkflowSpec,
    known_outputs: &HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    match condition {
        YamlCondition::Comparison(cmp) => {
            validate_field_path(&cmp.as_ref().field, spec, known_outputs)?;
        }
        YamlCondition::Combinator(c) => {
            validate_combinator_fields(c, spec, known_outputs)?;
        }
    }
    Ok(())
}

fn validate_combinator_fields(
    c: &YamlCombinator,
    spec: &WorkflowSpec,
    known_outputs: &HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    let all_children = c
        .all
        .iter()
        .flatten()
        .chain(c.any.iter().flatten())
        .chain(c.none.iter().flatten())
        .chain(c.one_of.iter().flatten());

    for child in all_children {
        validate_condition_fields(child, spec, known_outputs)?;
    }
    if let Some(ref inner) = c.not {
        validate_condition_fields(inner, spec, known_outputs)?;
    }
    Ok(())
}

/// Resolve a field path like `.inputs.foo` or `.outputs.bar` against the workflow schema.
fn validate_field_path(
    field: &str,
    spec: &WorkflowSpec,
    known_outputs: &HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    // If the spec has no inputs and no outputs schema, skip field validation
    // (schema-less workflow).
    if spec.inputs.is_empty() && spec.outputs.is_empty() {
        return Ok(());
    }

    let parts: Vec<&str> = field.split('.').collect();

    // Expect paths like ".inputs.x" or ".outputs.x" (leading dot is optional).
    let parts = if parts.first() == Some(&"") {
        &parts[1..] // skip leading empty from "."
    } else {
        &parts[..]
    };

    if parts.len() < 2 {
        return Err(YamlWorkflowError::Validation(format!(
            "Condition field path '{field}' must have at least two segments (e.g. '.inputs.name')"
        )));
    }

    match parts[0] {
        "inputs" => {
            let field_name = parts[1];
            if !spec.inputs.contains_key(field_name) {
                return Err(YamlWorkflowError::Validation(format!(
                    "Condition references unknown input field '{field_name}'. \
                     Available inputs: [{}]",
                    spec.inputs.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
        }
        "outputs" => {
            let field_name = parts[1];
            // Check both the declared output schema and step-produced outputs.
            if !spec.outputs.contains_key(field_name) && !known_outputs.contains(field_name) {
                return Err(YamlWorkflowError::Validation(format!(
                    "Condition references unknown output field '{field_name}'. \
                     Available outputs: [{}]",
                    spec.outputs.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
        }
        other => {
            return Err(YamlWorkflowError::Validation(format!(
                "Condition field path '{field}' must start with 'inputs' or 'outputs', got '{other}'"
            )));
        }
    }

    Ok(())
}

/// Validate operator type compatibility for condition comparisons.
pub fn validate_condition_types(
    condition: &YamlCondition,
    spec: &WorkflowSpec,
) -> Result<(), YamlWorkflowError> {
    match condition {
        YamlCondition::Comparison(cmp) => {
            validate_comparison_type(cmp.as_ref(), spec)?;
        }
        YamlCondition::Combinator(c) => {
            let all_children = c
                .all
                .iter()
                .flatten()
                .chain(c.any.iter().flatten())
                .chain(c.none.iter().flatten())
                .chain(c.one_of.iter().flatten());

            for child in all_children {
                validate_condition_types(child, spec)?;
            }
            if let Some(ref inner) = c.not {
                validate_condition_types(inner, spec)?;
            }
        }
    }
    Ok(())
}

/// Check that the operator used in a comparison is compatible with the field type.
fn validate_comparison_type(
    cmp: &YamlComparison,
    spec: &WorkflowSpec,
) -> Result<(), YamlWorkflowError> {
    // Resolve the field type from the schema.
    let field_type = resolve_field_type(&cmp.field, spec);
    let field_type = match field_type {
        Some(t) => t,
        // If we can't resolve the type (no schema), skip type checking.
        None => return Ok(()),
    };

    // Check operator compatibility.
    let has_gt = cmp.gt.is_some();
    let has_gte = cmp.gte.is_some();
    let has_lt = cmp.lt.is_some();
    let has_lte = cmp.lte.is_some();
    let has_contains = cmp.contains.is_some();
    let has_is_null = cmp.is_null == Some(true);
    let has_is_not_null = cmp.is_not_null == Some(true);

    // gt/gte/lt/lte only valid for number/integer types.
    if (has_gt || has_gte || has_lt || has_lte) && !is_numeric_type(&field_type) {
        return Err(YamlWorkflowError::Validation(format!(
            "Comparison operators gt/gte/lt/lte are only valid for number/integer types, \
             but field '{}' has type '{}'",
            cmp.field, field_type
        )));
    }

    // contains only valid for string/list types.
    if has_contains && !is_containable_type(&field_type) {
        return Err(YamlWorkflowError::Validation(format!(
            "Comparison operator 'contains' is only valid for string/list types, \
             but field '{}' has type '{}'",
            cmp.field, field_type
        )));
    }

    // is_null/is_not_null only valid for optional types.
    if (has_is_null || has_is_not_null) && !is_optional_type(&field_type) {
        return Err(YamlWorkflowError::Validation(format!(
            "Comparison operators is_null/is_not_null are only valid for optional types, \
             but field '{}' has type '{}'",
            cmp.field, field_type
        )));
    }

    Ok(())
}

/// Resolve a field's SchemaType from the workflow spec.
fn resolve_field_type(field: &str, spec: &WorkflowSpec) -> Option<SchemaType> {
    let parts: Vec<&str> = field.split('.').collect();
    let parts = if parts.first() == Some(&"") {
        &parts[1..]
    } else {
        &parts[..]
    };

    if parts.len() < 2 {
        return None;
    }

    let type_str = match parts[0] {
        "inputs" => spec.inputs.get(parts[1]),
        "outputs" => spec.outputs.get(parts[1]),
        _ => None,
    }?;

    parse_type_string(type_str).ok()
}

fn is_numeric_type(t: &SchemaType) -> bool {
    match t {
        SchemaType::Number | SchemaType::Integer | SchemaType::Any => true,
        SchemaType::Optional(inner) => is_numeric_type(inner),
        _ => false,
    }
}

fn is_containable_type(t: &SchemaType) -> bool {
    match t {
        SchemaType::String | SchemaType::List(_) | SchemaType::Any => true,
        SchemaType::Optional(inner) => is_containable_type(inner),
        _ => false,
    }
}

fn is_optional_type(t: &SchemaType) -> bool {
    matches!(t, SchemaType::Optional(_) | SchemaType::Any)
}

/// Detect output fields declared in `spec.outputs` that no step produces.
pub fn detect_unused_outputs(
    spec: &WorkflowSpec,
    known_outputs: &HashSet<String>,
) -> Result<(), YamlWorkflowError> {
    for output_name in spec.outputs.keys() {
        if !known_outputs.contains(output_name) {
            return Err(YamlWorkflowError::Validation(format!(
                "Declared output '{output_name}' is never produced by any step. \
                 Add an output data ref with name '{output_name}' to a step."
            )));
        }
    }
    Ok(())
}
