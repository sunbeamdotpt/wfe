use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A conditional step that branches execution based on a boolean condition.
#[derive(Default)]
pub struct IfStep {
    /// Condition.
    pub condition: bool,
}

#[async_trait]
impl StepBody for IfStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        let children_active = context
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if children_active {
            // Subsequent run: check if branch is complete.
            let mut scope = context.execution_pointer.scope.clone();
            scope.push(context.execution_pointer.id.clone());

            if context.workflow.is_branch_complete(&scope) {
                Ok(ExecutionResult::next())
            } else {
                Ok(ExecutionResult::persist(json!({"children_active": true})))
            }
        } else {
            // First run.
            if self.condition {
                Ok(ExecutionResult::branch(
                    vec![json!(null)],
                    Some(json!({"children_active": true})),
                ))
            } else {
                Ok(ExecutionResult::next())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionPointer, PointerStatus};
    use crate::primitives::test_helpers::*;

    #[tokio::test]
    async fn condition_true_first_run_branches() {
        let mut step = IfStep { condition: true };
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
    async fn condition_false_first_run_proceeds() {
        let mut step = IfStep { condition: false };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert!(result.branch_values.is_none());
    }

    #[tokio::test]
    async fn children_active_and_complete_proceeds() {
        let mut step = IfStep { condition: true };
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true}));

        let wf_step = default_step();

        // Create a workflow with child pointers that are all complete.
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
    async fn children_active_and_incomplete_persists() {
        let mut step = IfStep { condition: true };
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true}));

        let wf_step = default_step();

        // Create a workflow with a child pointer that is still running.
        let mut workflow = default_workflow();
        let mut child = ExecutionPointer::new(1);
        child.scope = vec![pointer.id.clone()];
        child.status = PointerStatus::Running;
        workflow.execution_pointers.push(child);

        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true}))
        );
    }
}
