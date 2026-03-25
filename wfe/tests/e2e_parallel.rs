use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use wfe::models::{
    ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};


/// Initial step before parallel.
#[derive(Default)]
struct StartStep;

#[async_trait]
impl StepBody for StartStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

/// A container step that branches its children and waits for all to complete.
/// This is our own Default-implementing version of SequenceStep.
#[derive(Default)]
struct ParallelContainerStep;

#[async_trait]
impl StepBody for ParallelContainerStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let children_active = ctx
            .persistence_data
            .and_then(|d| d.get("children_active"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if children_active {
            let mut scope = ctx.execution_pointer.scope.clone();
            scope.push(ctx.execution_pointer.id.clone());

            if ctx.workflow.is_branch_complete(&scope) {
                Ok(ExecutionResult::next())
            } else {
                Ok(ExecutionResult::persist(json!({"children_active": true})))
            }
        } else {
            // First run: branch for all children (one branch value spawns all children).
            Ok(ExecutionResult::branch(
                vec![json!(null)],
                Some(json!({"children_active": true})),
            ))
        }
    }
}

/// Step for branch A.
#[derive(Default)]
struct BranchAStep;

#[async_trait]
impl StepBody for BranchAStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

/// Step for branch B.
#[derive(Default)]
struct BranchBStep;

#[async_trait]
impl StepBody for BranchBStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_parallel_definition() -> WorkflowDefinition {
    // Manually construct:
    // Step 0: StartStep -> Step 1
    // Step 1: ParallelContainerStep (children: [2, 3]) -> (end)
    // Step 2: BranchAStep (child of 1)
    // Step 3: BranchBStep (child of 1)
    let mut def = WorkflowDefinition::new("parallel-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<StartStep>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });

    let mut step1 = WorkflowStep::new(1, std::any::type_name::<ParallelContainerStep>());
    step1.children = vec![2, 3];

    let step2 = WorkflowStep::new(2, std::any::type_name::<BranchAStep>());
    let step3 = WorkflowStep::new(3, std::any::type_name::<BranchBStep>());

    def.steps = vec![step0, step1, step2, step3];
    def
}

#[tokio::test]
async fn parallel_branches_both_complete() {
    let def = build_parallel_definition();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    host.register_step::<StartStep>().await;
    host.register_step::<ParallelContainerStep>().await;
    host.register_step::<BranchAStep>().await;
    host.register_step::<BranchBStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        "parallel-wf",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Both branch steps should have completed (they have non-empty scope).
    let branch_completions = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete && !p.scope.is_empty())
        .count();
    assert_eq!(branch_completions, 2, "Expected both parallel branches to complete");

    host.stop().await;
}
