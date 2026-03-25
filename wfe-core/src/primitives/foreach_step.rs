use async_trait::async_trait;
use serde_json::json;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that iterates over a collection, branching for each element.
pub struct ForEachStep {
    pub collection: Vec<serde_json::Value>,
    pub run_parallel: bool,
}

impl Default for ForEachStep {
    fn default() -> Self {
        Self {
            collection: Vec::new(),
            run_parallel: true,
        }
    }
}

#[async_trait]
impl StepBody for ForEachStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        if self.collection.is_empty() {
            return Ok(ExecutionResult::next());
        }

        if self.run_parallel {
            // Parallel: branch with all collection values at once.
            let children_active = context
                .persistence_data
                .and_then(|d| d.get("children_active"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if children_active {
                let mut scope = context.execution_pointer.scope.clone();
                scope.push(context.execution_pointer.id.clone());

                if context.workflow.is_branch_complete(&scope) {
                    Ok(ExecutionResult::next())
                } else {
                    Ok(ExecutionResult::persist(json!({"children_active": true})))
                }
            } else {
                Ok(ExecutionResult::branch(
                    self.collection.clone(),
                    Some(json!({"children_active": true})),
                ))
            }
        } else {
            // Sequential: process one item at a time using current_index.
            let current_index = context
                .persistence_data
                .and_then(|d| d.get("current_index"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let children_active = context
                .persistence_data
                .and_then(|d| d.get("children_active"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if children_active {
                // Check if current child is complete.
                let mut scope = context.execution_pointer.scope.clone();
                scope.push(context.execution_pointer.id.clone());

                if context.workflow.is_branch_complete(&scope) {
                    let next_index = current_index + 1;
                    if next_index >= self.collection.len() {
                        // All items processed.
                        Ok(ExecutionResult::next())
                    } else {
                        // Advance to next item.
                        Ok(ExecutionResult::branch(
                            vec![self.collection[next_index].clone()],
                            Some(json!({"children_active": true, "current_index": next_index})),
                        ))
                    }
                } else {
                    Ok(ExecutionResult::persist(
                        json!({"children_active": true, "current_index": current_index}),
                    ))
                }
            } else {
                // Start first item.
                Ok(ExecutionResult::branch(
                    vec![self.collection[current_index].clone()],
                    Some(json!({"children_active": true, "current_index": current_index})),
                ))
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
    async fn empty_collection_proceeds() {
        let mut step = ForEachStep {
            collection: vec![],
            run_parallel: true,
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
    }

    #[tokio::test]
    async fn parallel_branches_all_items() {
        let mut step = ForEachStep {
            collection: vec![json!(1), json!(2), json!(3)],
            run_parallel: true,
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(vec![json!(1), json!(2), json!(3)]));
    }

    #[tokio::test]
    async fn parallel_complete_proceeds() {
        let mut step = ForEachStep {
            collection: vec![json!(1), json!(2)],
            run_parallel: true,
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
    async fn sequential_starts_first_item() {
        let mut step = ForEachStep {
            collection: vec![json!("a"), json!("b"), json!("c")],
            run_parallel: false,
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(vec![json!("a")]));
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true, "current_index": 0}))
        );
    }

    #[tokio::test]
    async fn sequential_advances_to_next_item() {
        let mut step = ForEachStep {
            collection: vec![json!("a"), json!("b"), json!("c")],
            run_parallel: false,
        };
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true, "current_index": 0}));

        let wf_step = default_step();
        let mut workflow = default_workflow();
        let mut child = ExecutionPointer::new(1);
        child.scope = vec![pointer.id.clone()];
        child.status = PointerStatus::Complete;
        workflow.execution_pointers.push(child);

        let ctx = make_context(&pointer, &wf_step, &workflow);
        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(vec![json!("b")]));
        assert_eq!(
            result.persistence_data,
            Some(json!({"children_active": true, "current_index": 1}))
        );
    }

    #[tokio::test]
    async fn sequential_completes_after_last_item() {
        let mut step = ForEachStep {
            collection: vec![json!("a"), json!("b")],
            run_parallel: false,
        };
        let mut pointer = ExecutionPointer::new(0);
        pointer.persistence_data = Some(json!({"children_active": true, "current_index": 1}));

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
}
