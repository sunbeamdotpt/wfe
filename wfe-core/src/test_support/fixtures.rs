use chrono::Utc;

use crate::models::{Event, EventSubscription, ExecutionError, WorkflowInstance};

/// Create a sample `WorkflowInstance` for testing.
pub fn sample_workflow_instance() -> WorkflowInstance {
    WorkflowInstance::new("test-workflow", 1, serde_json::json!({"key": "value"}))
}

/// Create a sample `Event` for testing.
pub fn sample_event() -> Event {
    Event::new(
        "order.created",
        "order-123",
        serde_json::json!({"amount": 42}),
    )
}

/// Create a sample `EventSubscription` for testing.
pub fn sample_subscription() -> EventSubscription {
    EventSubscription::new("wf-1", 0, "ptr-1", "order.created", "order-123", Utc::now())
}

/// Create a sample `ExecutionError` for testing.
pub fn sample_execution_error() -> ExecutionError {
    ExecutionError::new("wf-1", "ptr-1", "something went wrong")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_workflow_instance_has_expected_fields() {
        let instance = sample_workflow_instance();
        assert_eq!(instance.workflow_definition_id, "test-workflow");
        assert_eq!(instance.version, 1);
    }

    #[test]
    fn sample_event_has_expected_fields() {
        let event = sample_event();
        assert_eq!(event.event_name, "order.created");
        assert_eq!(event.event_key, "order-123");
    }

    #[test]
    fn sample_subscription_has_expected_fields() {
        let sub = sample_subscription();
        assert_eq!(sub.workflow_id, "wf-1");
        assert_eq!(sub.event_name, "order.created");
        assert_eq!(sub.event_key, "order-123");
    }

    #[test]
    fn sample_execution_error_has_expected_fields() {
        let err = sample_execution_error();
        assert_eq!(err.workflow_id, "wf-1");
        assert_eq!(err.execution_pointer_id, "ptr-1");
        assert_eq!(err.message, "something went wrong");
    }
}
