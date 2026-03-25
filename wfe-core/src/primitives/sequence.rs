use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A container step that executes its children sequentially.
/// Completes when all children have finished.
#[derive(Default)]
pub struct SequenceStep;

#[async_trait]
impl StepBody for SequenceStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        let mut scope = context.execution_pointer.scope.clone();
        scope.push(context.execution_pointer.id.clone());

        if context.workflow.is_branch_complete(&scope) {
            Ok(ExecutionResult::next())
        } else {
            Ok(ExecutionResult::persist(json!({"children_active": true})))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionPointer, PointerStatus};
    use crate::primitives::test_helpers::*;

    #[tokio::test]
    async fn children_complete_proceeds() {
        let mut step = SequenceStep;
        let pointer = ExecutionPointer::new(0);
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
        let mut step = SequenceStep;
        let pointer = ExecutionPointer::new(0);
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

    #[tokio::test]
    async fn no_children_in_scope_proceeds() {
        let mut step = SequenceStep;
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();

        let ctx = make_context(&pointer, &wf_step, &workflow);
        let result = step.run(&ctx).await.unwrap();
        // No children in scope means is_branch_complete returns true (vacuously).
        assert!(result.proceed);
    }
}
