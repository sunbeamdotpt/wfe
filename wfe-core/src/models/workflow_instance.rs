use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::execution_pointer::ExecutionPointer;
use super::status::{PointerStatus, WorkflowStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    /// UUID — the primary key, always unique, never changes.
    pub id: String,
    /// Human-friendly unique name, e.g. "ci-42". Auto-assigned as
    /// `{definition_id}-{N}` via a per-definition monotonic counter when
    /// the caller does not supply an override. Used interchangeably with
    /// `id` in lookup APIs. Empty when the instance has not yet been
    /// persisted (the host fills it in before the first insert).
    pub name: String,
    /// UUID of the top-level ancestor workflow. `None` on the root
    /// (user-started) workflow; set to the parent's `root_workflow_id`
    /// (or the parent's `id` if the parent is itself a root) on every
    /// `SubWorkflowStep`-created child.
    ///
    /// Used by the Kubernetes executor to place all workflows in a tree
    /// under a single namespace — siblings started via `type: workflow`
    /// steps share the parent's namespace and any provisioned shared
    /// volume. Backends that don't care about workflow topology can
    /// ignore this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_workflow_id: Option<String>,
    pub workflow_definition_id: String,
    pub version: u32,
    pub description: Option<String>,
    pub reference: Option<String>,
    pub execution_pointers: Vec<ExecutionPointer>,
    pub next_execution: Option<i64>,
    pub status: WorkflowStatus,
    pub data: serde_json::Value,
    pub create_time: DateTime<Utc>,
    pub complete_time: Option<DateTime<Utc>>,
}

impl WorkflowInstance {
    pub fn new(
        workflow_definition_id: impl Into<String>,
        version: u32,
        data: serde_json::Value,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            // Filled in by WorkflowHost::start_workflow before persisting.
            name: String::new(),
            // None by default — caller (HostContextImpl) sets this when
            // starting a sub-workflow so children share the parent tree's
            // namespace/volume.
            root_workflow_id: None,
            workflow_definition_id: workflow_definition_id.into(),
            version,
            description: None,
            reference: None,
            execution_pointers: Vec::new(),
            next_execution: Some(0),
            status: WorkflowStatus::Runnable,
            data,
            create_time: Utc::now(),
            complete_time: None,
        }
    }

    /// Check if all execution pointers in a given scope have completed.
    pub fn is_branch_complete(&self, scope: &[String]) -> bool {
        self.execution_pointers
            .iter()
            .filter(|p| p.scope == scope)
            .all(|p| {
                matches!(
                    p.status,
                    PointerStatus::Complete
                        | PointerStatus::Skipped
                        | PointerStatus::Compensated
                        | PointerStatus::Cancelled
                        | PointerStatus::Failed
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn new_instance_defaults() {
        let instance = WorkflowInstance::new("test-workflow", 1, serde_json::json!({}));
        assert_eq!(instance.workflow_definition_id, "test-workflow");
        assert_eq!(instance.version, 1);
        assert_eq!(instance.status, WorkflowStatus::Runnable);
        assert_eq!(instance.next_execution, Some(0));
        assert!(instance.execution_pointers.is_empty());
        assert!(instance.complete_time.is_none());
    }

    #[test]
    fn is_branch_complete_empty_scope_returns_true() {
        let instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        assert!(instance.is_branch_complete(&[]));
    }

    #[test]
    fn is_branch_complete_all_complete() {
        let scope = vec!["parent-1".to_string()];
        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));

        let mut p1 = ExecutionPointer::new(0);
        p1.scope = scope.clone();
        p1.status = PointerStatus::Complete;

        let mut p2 = ExecutionPointer::new(1);
        p2.scope = scope.clone();
        p2.status = PointerStatus::Compensated;

        instance.execution_pointers = vec![p1, p2];
        assert!(instance.is_branch_complete(&scope));
    }

    #[test]
    fn is_branch_complete_with_active_pointer() {
        let scope = vec!["parent-1".to_string()];
        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));

        let mut p1 = ExecutionPointer::new(0);
        p1.scope = scope.clone();
        p1.status = PointerStatus::Complete;

        let mut p2 = ExecutionPointer::new(1);
        p2.scope = scope.clone();
        p2.status = PointerStatus::Running;

        instance.execution_pointers = vec![p1, p2];
        assert!(!instance.is_branch_complete(&scope));
    }

    #[test]
    fn is_branch_complete_ignores_different_scope() {
        let scope_a = vec!["parent-a".to_string()];
        let scope_b = vec!["parent-b".to_string()];
        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));

        let mut p1 = ExecutionPointer::new(0);
        p1.scope = scope_a.clone();
        p1.status = PointerStatus::Complete;

        let mut p2 = ExecutionPointer::new(1);
        p2.scope = scope_b.clone();
        p2.status = PointerStatus::Running;

        instance.execution_pointers = vec![p1, p2];
        assert!(instance.is_branch_complete(&scope_a));
    }

    #[test]
    fn serde_round_trip() {
        let instance = WorkflowInstance::new("my-workflow", 2, serde_json::json!({"key": "value"}));
        let json = serde_json::to_string(&instance).unwrap();
        let deserialized: WorkflowInstance = serde_json::from_str(&json).unwrap();
        assert_eq!(instance.id, deserialized.id);
        assert_eq!(
            instance.workflow_definition_id,
            deserialized.workflow_definition_id
        );
        assert_eq!(instance.version, deserialized.version);
        assert_eq!(instance.status, deserialized.status);
        assert_eq!(instance.data, deserialized.data);
    }
}
