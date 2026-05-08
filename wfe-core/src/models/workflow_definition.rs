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
}
