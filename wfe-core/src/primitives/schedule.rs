use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that schedules child execution after a delay.
#[derive(Default)]
pub struct ScheduleStep {
    pub interval: Duration,
}

#[async_trait]
impl StepBody for ScheduleStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        let children_active = context
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if children_active {
            // Children have been created, check if they are complete.
            let mut scope = context.execution_pointer.scope.clone();
            scope.push(context.execution_pointer.id.clone());

            if context.workflow.is_branch_complete(&scope) {
                Ok(ExecutionResult::next())
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
    async fn first_run_schedules_sleep() {
        let mut step = ScheduleStep {
            interval: Duration::from_secs(30),
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.sleep_for, Some(Duration::from_secs(30)));
        assert_eq!(result.persistence_data, Some(json!({"children_active": true})));
    }

    #[tokio::test]
    async fn children_complete_proceeds() {
        let mut step = ScheduleStep {
            interval: Duration::from_secs(30),
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
        assert!(result.proceed);
    }

    #[tokio::test]
    async fn children_incomplete_persists() {
        let mut step = ScheduleStep {
            interval: Duration::from_secs(30),
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
        assert_eq!(result.persistence_data, Some(json!({"children_active": true})));
    }
}
