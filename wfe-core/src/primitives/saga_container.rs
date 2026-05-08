use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A container step for saga transactions.
/// Manages child step execution and compensation on failure.
pub struct SagaContainerStep {
    /// Revert children after compensation.
    pub revert_children_after_compensation: bool,
}

impl Default for SagaContainerStep {
    fn default() -> Self {
        Self {
            revert_children_after_compensation: true,
        }
    }
}

#[async_trait]
impl StepBody for SagaContainerStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        let children_active = context
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let compensating = context
            .persistence_data
            .and_then(|d| d.get("compensating"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if compensating {
            // We are in compensation mode, check if compensation children are done.
            let mut scope = context.execution_pointer.scope.clone();
            scope.push(context.execution_pointer.id.clone());

            if context.workflow.is_branch_complete(&scope) {
                Ok(ExecutionResult::next())
            } else {
                Ok(ExecutionResult::persist(
                    json!({"children_active": true, "compensating": true}),
                ))
            }
        } else if children_active {
            let mut scope = context.execution_pointer.scope.clone();
            scope.push(context.execution_pointer.id.clone());

            // Check if compensation is needed by looking for do_compensate on the step.
            if context.step.do_compensate {
                // Trigger compensation.
                Ok(ExecutionResult::branch(
                    vec![json!(null)],
                    Some(json!({"children_active": true, "compensating": true})),
                ))
            } else if context.workflow.is_branch_complete(&scope) {
                // Normal completion.
                Ok(ExecutionResult::next())
            } else {
                Ok(ExecutionResult::persist(json!({"children_active": true})))
            }
        } else {
            // First run: branch for children.
            Ok(ExecutionResult::branch(
                vec![json!(null)],
                Some(json!({"children_active": true})),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionPointer, PointerStatus, WorkflowStep};
    use crate::primitives::test_helpers::*;

    #[tokio::test]
    async fn first_run_branches_for_children() {
        let mut step = SagaContainerStep::default();
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(vec![json!(null)]));
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true}))
        );
    }

    #[tokio::test]
    async fn normal_completion_when_children_done() {
        let mut step = SagaContainerStep::default();
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
    async fn compensation_triggered_when_do_compensate() {
        let mut step = SagaContainerStep::default();
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true}));

        let mut wf_step = WorkflowStep::new(0, "SagaContainer");
        wf_step.do_compensate = true;

        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(vec![json!(null)]));
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true, "compensating": true}))
        );
    }

    #[tokio::test]
    async fn compensation_complete_proceeds() {
        let mut step = SagaContainerStep::default();
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true, "compensating": true}));

        let wf_step = default_step();
        let mut workflow = default_workflow();
        let mut child = ExecutionPointer::new(1);
        child.scope = vec![pointer.id.clone()];
        child.status = PointerStatus::Compensated;
        workflow.execution_pointers.push(child);

        let ctx = make_context(&pointer, &wf_step, &workflow);
        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
    }
}
