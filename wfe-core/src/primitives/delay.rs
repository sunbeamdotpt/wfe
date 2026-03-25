use std::time::Duration;

use async_trait::async_trait;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that sleeps for a specified duration before proceeding.
pub struct DelayStep {
    pub duration: Duration,
}

impl Default for DelayStep {
    fn default() -> Self {
        Self {
            duration: Duration::ZERO,
        }
    }
}

#[async_trait]
impl StepBody for DelayStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        // Read duration from step_config if our field is zero.
        let duration = if self.duration == Duration::ZERO {
            context
                .step
                .step_config
                .as_ref()
                .and_then(|c| c.get("duration_millis"))
                .and_then(|v| v.as_u64())
                .map(Duration::from_millis)
                .unwrap_or(self.duration)
        } else {
            self.duration
        };
        Ok(ExecutionResult::sleep(duration, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionPointer;
    use crate::primitives::test_helpers::*;

    #[tokio::test]
    async fn returns_correct_sleep_duration() {
        let mut step = DelayStep {
            duration: Duration::from_secs(60),
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.sleep_for, Some(Duration::from_secs(60)));
        assert!(result.persistence_data.is_none());
    }

    #[tokio::test]
    async fn returns_zero_duration() {
        let mut step = DelayStep {
            duration: Duration::ZERO,
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert_eq!(result.sleep_for, Some(Duration::ZERO));
    }
}
