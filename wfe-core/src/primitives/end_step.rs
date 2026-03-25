use async_trait::async_trait;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A no-op marker step indicating the end of a workflow branch.
pub struct EndStep;

#[async_trait]
impl StepBody for EndStep {
    async fn run(&mut self, _context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionPointer;
    use crate::primitives::test_helpers::*;

    #[tokio::test]
    async fn always_returns_next() {
        let mut step = EndStep;
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert!(result.outcome_value.is_none());
        assert!(result.persistence_data.is_none());
    }
}
