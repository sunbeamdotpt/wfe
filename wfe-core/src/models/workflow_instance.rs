use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::execution_pointer::ExecutionPointer;
use super::status::{PointerStatus, WorkflowStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowInstance {
    pub id: String,
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
    pub fn new(workflow_definition_id: impl Into<String>, version: u32, data: serde_json::Value) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
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
        assert_eq!(instance.workflow_definition_id, deserialized.workflow_definition_id);
        assert_eq!(instance.version, deserialized.version);
        assert_eq!(instance.status, deserialized.status);
        assert_eq!(instance.data, deserialized.data);
    }
}
