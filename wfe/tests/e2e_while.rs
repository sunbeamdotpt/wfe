use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use wfe::models::{
    ExecutionResult, PointerStatus, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WhileData {
    target: i32,
}

/// A WhileStep-like step that loops `target` times using persistence_data.
#[derive(Default)]
struct CountingWhileStep;

#[async_trait]
impl StepBody for CountingWhileStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let children_active = ctx
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let iteration = ctx
            .persistence_data
            .and_then(|d| d.get("iteration"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let target = ctx
            .workflow
            .data
            .get("target")
            .and_then(|v| v.as_i64())
            .unwrap_or(3);

        if children_active {
            let mut scope = ctx.execution_pointer.scope.clone();
            scope.push(ctx.execution_pointer.id.clone());

            if ctx.workflow.is_branch_complete(&scope) {
                let new_iteration = iteration + 1;
                if new_iteration < target {
                    Ok(ExecutionResult::branch(
                        vec![json!(null)],
                        Some(json!({"children_active": true, "iteration": new_iteration})),
                    ))
                } else {
                    Ok(ExecutionResult::next())
                }
            } else {
                Ok(ExecutionResult::persist(
                    json!({"children_active": true, "iteration": iteration}),
                ))
            }
        } else if iteration < target {
            Ok(ExecutionResult::branch(
                vec![json!(null)],
                Some(json!({"children_active": true, "iteration": iteration})),
            ))
        } else {
            Ok(ExecutionResult::next())
        }
    }
}

/// A no-op child step representing one iteration body.
#[derive(Default)]
struct LoopBodyStep;

#[async_trait]
impl StepBody for LoopBodyStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_while_definition() -> WorkflowDefinition {
    // Step 0: CountingWhileStep (container, children: [1])
    // Step 1: LoopBodyStep (child of 0)
    let mut def = WorkflowDefinition::new("while-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<CountingWhileStep>());
    step0.children = vec![1];

    let step1 = WorkflowStep::new(1, std::any::type_name::<LoopBodyStep>());

    def.steps = vec![step0, step1];
    def
}

#[tokio::test]
async fn while_loop_runs_three_iterations() {
    let def = build_while_definition();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    host.register_step::<CountingWhileStep>().await;
    host.register_step::<LoopBodyStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let data = WhileData { target: 3 };

    let instance = run_workflow_sync(
        &host,
        "while-wf",
        1,
        serde_json::to_value(data).unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // The loop body should have run 3 times (3 child pointers with non-empty scope).
    let body_runs = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete && !p.scope.is_empty())
        .count();
    assert_eq!(body_runs, 3, "Expected 3 loop body executions");

    host.stop().await;
}
