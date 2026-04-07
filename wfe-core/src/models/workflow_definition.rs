use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::condition::StepCondition;
use super::error_behavior::ErrorBehavior;
use super::service::ServiceDefinition;

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
    pub version: u32,
    pub description: Option<String>,
    pub steps: Vec<WorkflowStep>,
    pub default_error_behavior: ErrorBehavior,
    #[serde(default, with = "super::option_duration_millis")]
    pub default_error_retry_interval: Option<Duration>,
    /// Infrastructure services required by this workflow (databases, caches, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceDefinition>,
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
    pub id: usize,
    pub name: Option<String>,
    pub external_id: Option<String>,
    pub step_type: String,
    pub children: Vec<usize>,
    pub outcomes: Vec<StepOutcome>,
    pub error_behavior: Option<ErrorBehavior>,
    pub compensation_step_id: Option<usize>,
    pub do_compensate: bool,
    #[serde(default)]
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
    pub next_step: usize,
    pub label: Option<String>,
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
