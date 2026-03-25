use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use wfe::models::{
    ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct IfData {
    condition: bool,
}

/// An IfStep-like step that reads the condition from workflow data.
#[derive(Default)]
struct DataDrivenIfStep;

#[async_trait]
impl StepBody for DataDrivenIfStep {
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
            let condition = ctx
                .workflow
                .data
                .get("condition")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if condition {
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

/// A child step that just completes.
#[derive(Default)]
struct ChildStep;

#[async_trait]
impl StepBody for ChildStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

/// A trailing step after the if block.
#[derive(Default)]
struct TrailingStep;

#[async_trait]
impl StepBody for TrailingStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_if_definition() -> WorkflowDefinition {
    // Step 0: DataDrivenIfStep (container, children: [1]) -> Step 2
    // Step 1: ChildStep (child of 0)
    // Step 2: TrailingStep
    let mut def = WorkflowDefinition::new("if-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<DataDrivenIfStep>());
    step0.children = vec![1];
    step0.outcomes.push(StepOutcome {
        next_step: 2,
        label: None,
        value: None,
    });

    let step1 = WorkflowStep::new(1, std::any::type_name::<ChildStep>());
    let step2 = WorkflowStep::new(2, std::any::type_name::<TrailingStep>());

    def.steps = vec![step0, step1, step2];
    def
}

fn make_host() -> wfe::WorkflowHost {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap()
}

#[tokio::test]
async fn if_true_runs_child_step() {
    let def = build_if_definition();
    let host = make_host();

    host.register_step::<DataDrivenIfStep>().await;
    host.register_step::<ChildStep>().await;
    host.register_step::<TrailingStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let data = IfData { condition: true };

    let instance = run_workflow_sync(
        &host,
        "if-wf",
        1,
        serde_json::to_value(data).unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // With condition=true, the child step should have run (it has non-empty scope).
    let child_step_ran = instance
        .execution_pointers
        .iter()
        .any(|p| p.status == PointerStatus::Complete && !p.scope.is_empty());
    assert!(
        child_step_ran,
        "Expected child step inside if-branch to run"
    );

    host.stop().await;
}

#[tokio::test]
async fn if_false_skips_child_step() {
    let def = build_if_definition();
    let host = make_host();

    host.register_step::<DataDrivenIfStep>().await;
    host.register_step::<ChildStep>().await;
    host.register_step::<TrailingStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let data = IfData { condition: false };

    let instance = run_workflow_sync(
        &host,
        "if-wf",
        1,
        serde_json::to_value(data).unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // With condition=false, no child step in scope should have run.
    let child_step_ran = instance
        .execution_pointers
        .iter()
        .any(|p| p.status == PointerStatus::Complete && !p.scope.is_empty());
    assert!(
        !child_step_ran,
        "Expected no child steps to run when condition is false"
    );

    host.stop().await;
}
