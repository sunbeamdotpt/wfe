use async_trait::async_trait;
use chrono::Utc;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that waits for an external event before proceeding.
#[derive(Default)]
pub struct WaitForStep {
    /// Event name.
    pub event_name: String,
    /// Event key.
    pub event_key: String,
}

#[async_trait]
impl StepBody for WaitForStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        // If event data has arrived, proceed.
        if context.execution_pointer.event_data.is_some() {
            return Ok(ExecutionResult::next());
        }

        // Read event_name/event_key from step_config if our fields are empty.
        let event_name = if self.event_name.is_empty() {
            context
                .step
                .step_config
                .as_ref()
                .and_then(|c| c.get("event_name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        } else {
            self.event_name.clone()
        };
        let event_key = if self.event_key.is_empty() {
            context
                .step
                .step_config
                .as_ref()
                .and_then(|c| c.get("event_key"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        } else {
            self.event_key.clone()
        };

        // Otherwise, subscribe and wait for the event.
        Ok(ExecutionResult::wait_for_event(
            event_name,
            event_key,
            Utc::now(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionPointer;
    use crate::primitives::test_helpers::*;
    use serde_json::json;

    #[tokio::test]
    async fn first_run_waits_for_event() {
        let mut step = WaitForStep {
            event_name: "order.completed".into(),
            event_key: "order-123".into(),
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.event_name.as_deref(), Some("order.completed"));
        assert_eq!(result.event_key.as_deref(), Some("order-123"));
        assert!(result.event_as_of.is_some());
    }

    #[tokio::test]
    async fn event_arrived_proceeds() {
        let mut step = WaitForStep {
            event_name: "order.completed".into(),
            event_key: "order-123".into(),
        };
        let mut pointer = ExecutionPointer::new(0);
        pointer.event_data = Some(json!({"status": "done"}));
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert!(result.event_name.is_none());
    }
}
