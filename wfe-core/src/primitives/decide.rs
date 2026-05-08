use async_trait::async_trait;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A decision step that returns an outcome value for routing.
#[derive(Default)]
pub struct DecideStep {
    /// Expression value.
    pub expression_value: serde_json::Value,
}

#[async_trait]
impl StepBody for DecideStep {
    async fn run(&mut self, _context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        Ok(ExecutionResult::outcome(self.expression_value.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionPointer;
    use crate::primitives::test_helpers::*;
    use serde_json::json;

    #[tokio::test]
    async fn returns_correct_outcome_value() {
        let mut step = DecideStep {
            expression_value: json!("route_a"),
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert_eq!(result.outcome_value, Some(json!("route_a")));
    }

    #[tokio::test]
    async fn returns_numeric_outcome() {
        let mut step = DecideStep {
            expression_value: json!(42),
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert_eq!(result.outcome_value, Some(json!(42)));
    }
}
