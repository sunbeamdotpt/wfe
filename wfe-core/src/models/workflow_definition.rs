use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::condition::StepCondition;
use super::error_behavior::ErrorBehavior;
use super::service::ServiceDefinition;

/// Declaration of a volume that persists across every step in a workflow
/// run, including sub-workflows started via `type: workflow` steps. Backends
/// that support it (currently just Kubernetes) provision a single volume
/// per top-level workflow instance and mount it on every step container at
/// `mount_path`. Sub-workflows see the same volume because they share the
/// parent's isolation domain (namespace, in the K8s case).
///
/// Declared once on the top-level workflow (e.g. `ci`) that orchestrates
/// the sub-workflows. Declarations on non-root workflows are ignored in
/// favor of the root's declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedVolume {
    /// Absolute path the volume is mounted at inside every step container.
    /// Typical value: `/workspace`.
    pub mount_path: String,
    /// Optional size override (e.g. `"20Gi"`). When unset the backend falls
    /// back to its configured default (ClusterConfig::default_shared_volume_size
    /// for the Kubernetes executor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

impl Default for SharedVolume {
    fn default() -> Self {
        Self {
            mount_path: "/workspace".to_string(),
            size: None,
        }
    }
}

/// A compiled workflow definition ready for execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    /// Stable slug used as the primary key (e.g. "ci", "checkout"). Must be
    /// unique within a host. Referenced by other workflows, webhooks, and
    /// clients when starting new instances.
    pub id: String,
    /// Optional human-friendly display name surfaced in UIs, listings, and
    /// logs (e.g. "Continuous Integration"). Falls back to `id` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Version.
    pub version: u32,
    /// Description.
    pub description: Option<String>,
    /// Steps.
    pub steps: Vec<WorkflowStep>,
    /// Default error behavior.
    pub default_error_behavior: ErrorBehavior,
    #[serde(default, with = "super::option_duration_millis")]
    /// Default error retry interval.
    pub default_error_retry_interval: Option<Duration>,
    /// Infrastructure services required by this workflow (databases, caches, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceDefinition>,
    /// When set, the backend provisions a single persistent volume for the
    /// top-level workflow instance and mounts it on every step container.
    /// All sub-workflows inherit the same volume through their shared
    /// namespace/isolation domain. Sub-workflow declarations are ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_volume: Option<SharedVolume>,
}

impl WorkflowDefinition {
    /// Create a new empty workflow definition with the given id and version.
    pub fn new(id: impl Into<String>, version: u32) -> Self {
        Self {
            id: id.into(),
            name: None,
            version,
            description: None,
            steps: Vec::new(),
            default_error_behavior: ErrorBehavior::default(),
            default_error_retry_interval: None,
            services: Vec::new(),
            shared_volume: None,
        }
    }

    /// Return the display name when set, otherwise fall back to the slug id.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Write a Graphviz DOT representation of this workflow to the given writer.
    ///
    /// The output is a directed graph showing:
    ///
    /// | Visual element | Meaning |
    /// |----------------|---------|
    /// | **Node** | One per step, labeled with name and type |
    /// | **Solid edge** | Sequential execution flow (`outcomes`) |
    /// | **Dotted gray edge** | Container/child relationship |
    /// | **Dashed red edge** | Compensation path |
    ///
    /// Container steps (parallel, if, while, foreach, saga, decide) are drawn
    /// with a distinct fill colour so you can spot control-flow primitives at a
    /// glance.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let definition = WorkflowBuilder::<MyData>::new()
    ///     .start_with::<StepA>()
    ///     .then::<StepB>()
    ///     .end_workflow()
    ///     .build("demo", 1);
    ///
    /// let dot = definition.to_dot();
    /// std::fs::write("workflow.dot", &dot).unwrap();
    /// ```
    pub fn write_dot<W: std::fmt::Write>(&self, w: &mut W) -> std::fmt::Result {
        let display = self.display_name();
        writeln!(w, "digraph {} {{", dot_escape_id(&self.id))?;
        writeln!(w, "    label={}", dot_escape(&format!("{} v{}", display, self.version)))?;
        writeln!(w, "    labelloc=t")?;
        writeln!(w, "    fontsize=14")?;
        writeln!(w, "    fontname=\"Helvetica-Bold\"")?;
        writeln!(w, "    rankdir=TB")?;
        writeln!(w, "    graph [fontname=\"Helvetica\", fontsize=10, bgcolor=\"white\", margin=20]")?;
        writeln!(
            w,
            "    node [fontname=\"Helvetica\", shape=box, style=\"rounded,filled\", fillcolor=\"#f3f4f6\", fontsize=9, penwidth=1.0]"
        )?;
        writeln!(
            w,
            "    edge [fontname=\"Helvetica\", fontsize=8, color=\"#374151\", penwidth=1.0]"
        )?;
        writeln!(w)?;

        // --- nodes ---
        for step in &self.steps {
            let label = self.step_dot_label(step);
            let fill = step_fill_color(step);
            let shape = step_shape(step);
            let mut attrs: Vec<String> = Vec::new();

            attrs.push(format!("label={}", label));
            attrs.push(format!("shape={}", shape));
            attrs.push(format!("fillcolor=\"{}\"", fill));

            if step.when.is_some() {
                attrs.push("style=\"rounded,filled,dashed\"".to_string());
                attrs.push("color=\"#059669\"".to_string()); // green border for condition
            }

            if step.error_behavior.is_some() {
                attrs.push("penwidth=1.5".to_string());
            }

            write!(w, "    N{} [", step.id)?;
            write!(w, "{}", attrs.join(", "))?;
            writeln!(w, "];")?;
        }

        writeln!(w)?;

        // --- outcome edges (sequential flow) ---
        for step in &self.steps {
            for outcome in &step.outcomes {
                let mut edge_attrs: Vec<String> = Vec::new();

                if let Some(ref lbl) = outcome.label {
                    edge_attrs.push(format!("label={}", dot_escape(lbl)));
                } else if let Some(ref val) = outcome.value {
                    let txt = truncate(&format!("{}", val), 40);
                    edge_attrs.push(format!("label={}", dot_escape(&txt)));
                }

                write!(w, "    N{} -> N{}", step.id, outcome.next_step)?;
                if !edge_attrs.is_empty() {
                    write!(w, " [{}]", edge_attrs.join(", "))?;
                }
                writeln!(w, ";")?;
            }
        }

        // --- child edges (containment) ---
        for step in &self.steps {
            for &child_id in &step.children {
                writeln!(
                    w,
                    "    N{} -> N{} [style=dotted, color=\"#9ca3af\", arrowhead=none, penwidth=1.2];",
                    step.id, child_id
                )?;
            }
        }

        // --- compensation edges ---
        for step in &self.steps {
            if let Some(comp_id) = step.compensation_step_id {
                writeln!(
                    w,
                    "    N{} -> N{} [style=dashed, color=\"#dc2626\", label=\"compensate\", fontcolor=\"#dc2626\", penwidth=1.2];",
                    step.id, comp_id
                )?;
            }
        }

        writeln!(w, "}}")?;
        Ok(())
    }

    /// Return a Graphviz DOT representation as a `String`.
    pub fn to_dot(&self) -> String {
        let mut buf = String::new();
        self.write_dot(&mut buf)
            .expect("writing to String is infallible");
        buf
    }

    // ------------------------------------------------------------------
    // helpers
    // ------------------------------------------------------------------

    fn step_dot_label(&self, step: &WorkflowStep) -> String {
        let mut lines: Vec<String> = Vec::new();

        // Name (if set)
        if let Some(ref name) = step.name {
            lines.push(format!(
                "<TR><TD><B>{}</B></TD></TR>",
                html_escape(name)
            ));
        }

        // Type (always shown, cleaned up)
        let type_name = clean_type_name(&step.step_type);
        lines.push(format!(
            "<TR><TD><FONT POINT-SIZE=\"8\" COLOR=\"#6b7280\">{}</FONT></TD></TR>",
            html_escape(&type_name)
        ));

        // Condition badge
        if step.when.is_some() {
            lines.push(
                "<TR><TD><FONT POINT-SIZE=\"7\" COLOR=\"#059669\">[when]</FONT></TD></TR>"
                    .to_string(),
            );
        }

        // Error behaviour badge
        if let Some(ref eb) = step.error_behavior {
            lines.push(format!(
                "<TR><TD><FONT POINT-SIZE=\"7\" COLOR=\"#dc2626\">[{:?}]</FONT></TD></TR>",
                eb
            ));
        }

        format!(
            "<\n  <TABLE BORDER=\"0\" CELLBORDER=\"0\" CELLSPACING=\"1\" CELLPADDING=\"2\">\n    {}\n  </TABLE>\n>",
            lines.join("\n    ")
        )
    }
}

/// A single step in a workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Id.
    pub id: usize,
    /// Name.
    pub name: Option<String>,
    /// External id.
    pub external_id: Option<String>,
    /// Step type.
    pub step_type: String,
    /// Children.
    pub children: Vec<usize>,
    /// Outcomes.
    pub outcomes: Vec<StepOutcome>,
    /// Error behavior.
    pub error_behavior: Option<ErrorBehavior>,
    /// Compensation step id.
    pub compensation_step_id: Option<usize>,
    /// Do compensate.
    pub do_compensate: bool,
    #[serde(default)]
    /// Saga.
    pub saga: bool,
    /// Serializable configuration for primitive steps (e.g. event_name, duration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_config: Option<serde_json::Value>,
    /// Optional condition that must evaluate to true for this step to execute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<StepCondition>,
}

impl WorkflowStep {
    /// Create a new step with the given id and type.
    pub fn new(id: usize, step_type: impl Into<String>) -> Self {
        Self {
            id,
            name: None,
            external_id: None,
            step_type: step_type.into(),
            children: Vec::new(),
            outcomes: Vec::new(),
            error_behavior: None,
            compensation_step_id: None,
            do_compensate: false,
            saga: false,
            step_config: None,
            when: None,
        }
    }

    /// Extract artifact input declarations from the step's raw configuration.
    ///
    /// Looks for `config.inputs` (a map of name → target path) and
    /// `config.input` (a single artifact name, used by buildkit). Returns
    /// a map of input name → target path. For `config.input`, the target
    /// path is empty since buildkit uses the artifact as a build context
    /// override rather than a mount point.
    pub fn artifact_inputs(&self) -> HashMap<String, String> {
        let mut result = HashMap::new();

        let config = match self.step_config.as_ref() {
            Some(c) => c,
            None => return result,
        };

        // Map form: inputs: { repo: /workspace/repo }
        if let Some(inputs) = config.get("inputs").and_then(|v| v.as_object()) {
            for (k, v) in inputs {
                if let Some(s) = v.as_str() {
                    result.insert(k.clone(), s.to_string());
                }
            }
        }

        // Single form: input: repo (buildkit)
        if let Some(input) = config.get("input").and_then(|v| v.as_str()) {
            result.insert(input.to_string(), String::new());
        }

        result
    }
}

// ------------------------------------------------------------------
// DOT rendering helpers
// ------------------------------------------------------------------

/// Escape a string for use inside DOT double-quoted identifiers / labels.
fn dot_escape(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    format!("\"{}\"", escaped)
}

/// Escape a string for use as a DOT graph ID. Same rules as `dot_escape`.
fn dot_escape_id(s: &str) -> String {
    dot_escape(s)
}

/// Escape characters that are special inside DOT HTML-like labels.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Strip common crate-path prefixes and leave just the type name.
fn clean_type_name(type_name: &str) -> String {
    // First strip known prefixes
    let mut s = type_name.to_string();
    for prefix in &[
        "wfe_core::primitives::",
        "wfe_core::",
        "crate::workflows::primitives::",
        "crate::workflows::steps::",
        "crate::workflows::up::steps::",
        "crate::workflows::down::steps::",
        "crate::",
    ] {
        s = s.replace(prefix, "");
    }
    // If there are still '::' segments, keep only the last one (the type name itself)
    if let Some(pos) = s.rfind("::") {
        s.split_off(pos + 2)
    } else {
        s
    }
}

/// Return a Graphviz fill colour for a step based on its type.
fn step_fill_color(step: &WorkflowStep) -> &'static str {
    if step.saga {
        return "#ede9fe"; // light purple
    }

    let t = &step.step_type;
    if t.contains("SequenceStep") {
        "#dbeafe" // light blue   – parallel container
    } else if t.contains("IfStep") {
        "#d1fae5" // light green  – conditional
    } else if t.contains("WhileStep") {
        "#fef3c7" // light yellow – loop
    } else if t.contains("ForEachStep") {
        "#ffedd5" // light orange – iteration
    } else if t.contains("SagaContainerStep") {
        "#ede9fe" // light purple – saga
    } else if t.contains("DecideStep") {
        "#ccfbf1" // light teal   – multi-way branch
    } else if t.contains("WaitForStep") {
        "#fce7f3" // light pink   – event wait
    } else if t.contains("DelayStep") || t.contains("ScheduleStep") {
        "#f3e8ff" // light violet – time-based
    } else if t.contains("EndStep") {
        "#e5e7eb" // medium gray  – terminal
    } else if t.contains("SubWorkflowStep") {
        "#e0f2fe" // light sky    – sub-workflow
    } else {
        "#f3f4f6" // light gray   – default leaf step
    }
}

/// Return a Graphviz shape for a step.
fn step_shape(step: &WorkflowStep) -> &'static str {
    let t = &step.step_type;
    if t.contains("EndStep") {
        "oval"
    } else if t.contains("DecideStep") {
        "diamond"
    } else if t.contains("SequenceStep")
        || t.contains("IfStep")
        || t.contains("WhileStep")
        || t.contains("ForEachStep")
        || t.contains("SagaContainerStep")
    {
        "box3d"
    } else {
        "box"
    }
}

/// Truncate a string to `max_len` characters, appending "…" if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len.saturating_sub(1)])
    }
}

/// Routing outcome from a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepOutcome {
    /// Next step.
    pub next_step: usize,
    /// Label.
    pub label: Option<String>,
    /// Value.
    pub value: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn definition_defaults() {
        let def = WorkflowDefinition::new("test-workflow", 1);
        assert_eq!(def.id, "test-workflow");
        assert_eq!(def.version, 1);
        assert!(def.steps.is_empty());
        assert_eq!(def.default_error_behavior, ErrorBehavior::default());
        assert!(def.default_error_retry_interval.is_none());
    }

    #[test]
    fn step_defaults() {
        let step = WorkflowStep::new(0, "MyStep");
        assert_eq!(step.id, 0);
        assert_eq!(step.step_type, "MyStep");
        assert!(step.children.is_empty());
        assert!(step.outcomes.is_empty());
        assert!(step.error_behavior.is_none());
        assert!(step.compensation_step_id.is_none());
    }

    #[test]
    fn definition_serde_round_trip() {
        let mut def = WorkflowDefinition::new("wf", 3);
        let mut step = WorkflowStep::new(0, "StepA");
        step.outcomes.push(StepOutcome {
            next_step: 1,
            label: Some("next".into()),
            value: None,
        });
        def.steps.push(step);
        def.steps.push(WorkflowStep::new(1, "StepB"));

        let json = serde_json::to_string(&def).unwrap();
        let deserialized: WorkflowDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def.id, deserialized.id);
        assert_eq!(def.steps.len(), deserialized.steps.len());
        assert_eq!(def.steps[0].outcomes[0].next_step, 1);
    }

    // ------------------------------------------------------------------
    // DOT output tests
    // ------------------------------------------------------------------

    #[test]
    fn dot_empty_workflow() {
        let def = WorkflowDefinition::new("empty", 1);
        let dot = def.to_dot();
        assert!(dot.starts_with("digraph \"empty\" {"));
        assert!(dot.contains("label=\"empty v1\""));
        assert!(dot.ends_with("}\n"));
    }

    #[test]
    fn dot_simple_chain() {
        let mut def = WorkflowDefinition::new("chain", 1);
        let mut step0 = WorkflowStep::new(0, "StepA");
        step0.outcomes.push(StepOutcome {
            next_step: 1,
            label: None,
            value: None,
        });
        def.steps.push(step0);
        def.steps.push(WorkflowStep::new(1, "StepB"));

        let dot = def.to_dot();
        assert!(dot.contains("N0 ["));
        assert!(dot.contains("N1 ["));
        assert!(dot.contains("N0 -> N1"));
    }

    #[test]
    fn dot_parallel_branches() {
        let mut def = WorkflowDefinition::new("parallel", 1);
        // Step 0: leaf
        let mut step0 = WorkflowStep::new(0, "Start");
        step0.outcomes.push(StepOutcome {
            next_step: 1,
            label: None,
            value: None,
        });
        def.steps.push(step0);

        // Step 1: SequenceStep (parallel container)
        let mut seq = WorkflowStep::new(1, "wfe_core::primitives::sequence::SequenceStep");
        seq.children = vec![2, 3];
        seq.outcomes.push(StepOutcome {
            next_step: 4,
            label: None,
            value: None,
        });
        def.steps.push(seq);

        // Steps 2 and 3: branch bodies
        let mut step2 = WorkflowStep::new(2, "BranchA");
        step2.outcomes.push(StepOutcome {
            next_step: 4,
            label: None,
            value: None,
        });
        def.steps.push(step2);

        let mut step3 = WorkflowStep::new(3, "BranchB");
        step3.outcomes.push(StepOutcome {
            next_step: 4,
            label: None,
            value: None,
        });
        def.steps.push(step3);

        // Step 4: end
        def.steps.push(WorkflowStep::new(4, "End"));

        let dot = def.to_dot();
        // Container should be box3d and light blue
        assert!(dot.contains("shape=box3d"));
        assert!(dot.contains("fillcolor=\"#dbeafe\""));
        // Child edges should be dotted
        assert!(dot.contains("N1 -> N2 [style=dotted"));
        assert!(dot.contains("N1 -> N3 [style=dotted"));
        // Sequential flow
        assert!(dot.contains("N0 -> N1"));
        assert!(dot.contains("N1 -> N4"));
    }

    #[test]
    fn dot_with_names() {
        let mut def = WorkflowDefinition::new("named", 1);
        let mut step = WorkflowStep::new(0, "sunbeam_sdk::workflows::up::steps::EnsureCilium");
        step.name = Some("ensure-cilium".into());
        def.steps.push(step);

        let dot = def.to_dot();
        // Name should appear in the label
        assert!(dot.contains("ensure-cilium"));
        // Type should be cleaned to just the basename
        assert!(dot.contains("EnsureCilium"));
        // Should NOT contain the full path
        assert!(!dot.contains("sunbeam_sdk::workflows::up::steps::"));
    }

    #[test]
    fn dot_compensation_edge() {
        let mut def = WorkflowDefinition::new("saga", 1);
        let mut step0 = WorkflowStep::new(0, "DoWork");
        step0.compensation_step_id = Some(1);
        def.steps.push(step0);
        def.steps.push(WorkflowStep::new(1, "UndoWork"));

        let dot = def.to_dot();
        assert!(dot.contains("N0 -> N1 [style=dashed, color=\"#dc2626\", label=\"compensate\""));
    }

    #[test]
    fn dot_condition_badge() {
        let mut def = WorkflowDefinition::new("conditional", 1);
        let mut step = WorkflowStep::new(0, "MaybeRun");
        step.when = Some(StepCondition::Comparison(
            crate::models::FieldComparison {
                field: ".skip".to_string(),
                operator: crate::models::ComparisonOp::Equals,
                value: Some(serde_json::json!(false)),
            },
        ));
        def.steps.push(step);

        let dot = def.to_dot();
        // Should have green border for condition
        assert!(dot.contains("color=\"#059669\""));
        assert!(dot.contains("[when]"));
    }

    #[test]
    fn dot_error_badge() {
        let mut def = WorkflowDefinition::new("err", 1);
        let mut step = WorkflowStep::new(0, "RiskyStep");
        step.error_behavior = Some(ErrorBehavior::Terminate);
        def.steps.push(step);

        let dot = def.to_dot();
        // Should have thicker border
        assert!(dot.contains("penwidth=1.5"));
        // Should show the error behaviour
        assert!(dot.contains("Terminate"));
    }

    #[test]
    fn clean_type_name_strips_prefixes() {
        assert_eq!(clean_type_name("wfe_core::primitives::sequence::SequenceStep"), "SequenceStep");
        assert_eq!(clean_type_name("crate::workflows::steps::EnsureCilium"), "EnsureCilium");
        assert_eq!(clean_type_name("MyCustomStep"), "MyCustomStep");
        assert_eq!(clean_type_name("a::b::c::DeepStep"), "DeepStep");
    }

    #[test]
    fn html_escape_special_chars() {
        assert_eq!(html_escape("foo & bar"), "foo &amp; bar");
        assert_eq!(html_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }
}
