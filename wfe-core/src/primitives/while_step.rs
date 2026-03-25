use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A looping step that repeats its children while a condition is true.
pub struct WhileStep {
    pub condition: bool,
}

#[async_trait]
impl StepBody for WhileStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        let children_active = context
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if children_active {
            // Subsequent run: check if the current iteration is complete.
            let mut scope = context.execution_pointer.scope.clone();
            scope.push(context.execution_pointer.id.clone());

            if context.workflow.is_branch_complete(&scope) {
                // Iteration complete. Re-evaluate condition.
                if self.condition {
                    // Start a new iteration.
                    Ok(ExecutionResult::branch(
                        vec![json!(null)],
                        Some(json!({"children_active": true})),
                    ))
                } else {
                    Ok(ExecutionResult::next())
                }
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
        let mut step = WhileStep { condition: true };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(vec![json!(null)]));
        assert_eq!(result.persistence_data, Some(json!({"children_active": true})));
    }

    #[tokio::test]
    async fn condition_false_first_run_proceeds() {
        let mut step = WhileStep { condition: false };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
    }

    #[tokio::test]
    async fn children_complete_and_condition_true_re_branches() {
        let mut step = WhileStep { condition: true };
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
        assert_eq!(result.branch_values, Some(vec![json!(null)]));
    }

    #[tokio::test]
    async fn children_complete_and_condition_false_proceeds() {
        let mut step = WhileStep { condition: false };
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
        let mut step = WhileStep { condition: true };
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
        assert!(result.branch_values.is_none());
        assert_eq!(result.persistence_data, Some(json!({"children_active": true})));
    }
}
