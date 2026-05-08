use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that repeatedly schedules child execution at an interval
/// until a stop condition is met.
#[derive(Default)]
pub struct RecurStep {
    /// Interval.
    pub interval: Duration,
    /// Stop condition.
    pub stop_condition: bool,
}

#[async_trait]
impl StepBody for RecurStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        if self.stop_condition {
            return Ok(ExecutionResult::next());
        }

        let children_active = context
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if children_active {
            let mut scope = context.execution_pointer.scope.clone();
            scope.push(context.execution_pointer.id.clone());

            if context.workflow.is_branch_complete(&scope) {
                // Re-arm: sleep again for the next iteration.
                Ok(ExecutionResult::sleep(
                    self.interval,
                    Some(json!({"children_active": true})),
                ))
            } else {
                Ok(ExecutionResult::persist(json!({"children_active": true})))
            }
        } else {
            // First run: sleep for the interval, then create children.
            Ok(ExecutionResult::sleep(
                self.interval,
                Some(json!({"children_active": true})),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionPointer, PointerStatus};
    use crate::primitives::test_helpers::*;

    #[tokio::test]
    async fn stop_condition_true_proceeds() {
        let mut step = RecurStep {
            interval: Duration::from_secs(10),
            stop_condition: true,
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
    }

    #[tokio::test]
    async fn first_run_sleeps() {
        let mut step = RecurStep {
            interval: Duration::from_secs(10),
            stop_condition: false,
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.sleep_for, Some(Duration::from_secs(10)));
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true}))
        );
    }

    #[tokio::test]
    async fn children_complete_re_arms() {
        let mut step = RecurStep {
            interval: Duration::from_secs(10),
            stop_condition: false,
        };
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true}));

        let wf_step = default_step();
        let mut workflow = default_workflow();
        let mut child = ExecutionPointer::new(1);
        child.scope = vec![pointer.id.clone()];
        child.status = PointerStatus::Complete;
        workflow.execution_pointers.push(child);

        let ctx = make_context(&pointer, &wf_step, &workflow);
        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.sleep_for, Some(Duration::from_secs(10)));
    }

    #[tokio::test]
    async fn children_incomplete_persists() {
        let mut step = RecurStep {
            interval: Duration::from_secs(10),
            stop_condition: false,
        };
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true}));

        let wf_step = default_step();
        let mut workflow = default_workflow();
        let mut child = ExecutionPointer::new(1);
        child.scope = vec![pointer.id.clone()];
        child.status = PointerStatus::Running;
        workflow.execution_pointers.push(child);

        let ctx = make_context(&pointer, &wf_step, &workflow);
        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert!(result.sleep_for.is_none());
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true}))
        );
    }
}
