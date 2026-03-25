use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use wfe::models::{
    ErrorBehavior, ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition,
    WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};


/// A step that fails on the first attempt but succeeds on retry.
/// Uses retry_count on the execution pointer to track attempts.
#[derive(Default)]
struct FailOnceStep;

#[async_trait]
impl StepBody for FailOnceStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        if ctx.execution_pointer.retry_count == 0 {
            Err(wfe_core::WfeError::StepExecution(
                "Simulated failure on first attempt".into(),
            ))
        } else {
            Ok(ExecutionResult::next())
        }
    }
}

/// A step that always fails.
#[derive(Default)]
struct AlwaysFailStep;

#[async_trait]
impl StepBody for AlwaysFailStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Err(wfe_core::WfeError::StepExecution(
            "Permanent failure".into(),
        ))
    }
}

/// A simple passthrough step.
#[derive(Default)]
struct PassStep;

#[async_trait]
impl StepBody for PassStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
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
async fn retry_succeeds_on_second_attempt() {
    let host = make_host();

    // Build definition with Retry error behavior (very short interval).
    let mut def = WorkflowDefinition::new("retry-wf", 1);
    let mut step0 = WorkflowStep::new(0, std::any::type_name::<FailOnceStep>());
    step0.error_behavior = Some(ErrorBehavior::Retry {
        interval: Duration::from_millis(10),
        max_retries: 0,
    });
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<PassStep>());
    def.steps = vec![step0, step1];

    host.register_step::<FailOnceStep>().await;
    host.register_step::<PassStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        "retry-wf",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // The first step should have a retry_count > 0.
    let retried_pointer = instance
        .execution_pointers
        .iter()
        .find(|p| p.retry_count > 0);
    assert!(
        retried_pointer.is_some(),
        "Expected a pointer with retry_count > 0"
    );

    host.stop().await;
}

#[tokio::test]
async fn suspend_error_behavior_suspends_workflow() {
    let host = make_host();

    let mut def = WorkflowDefinition::new("suspend-err-wf", 1);
    let mut step0 = WorkflowStep::new(0, std::any::type_name::<AlwaysFailStep>());
    step0.error_behavior = Some(ErrorBehavior::Suspend);
    def.steps = vec![step0];

    host.register_step::<AlwaysFailStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let id = host
        .start_workflow("suspend-err-wf", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Poll until the workflow is suspended.
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("Workflow did not become Suspended within timeout");
        }
        let instance = host.get_workflow(&id).await.unwrap();
        if instance.status == WorkflowStatus::Suspended {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let instance = host.get_workflow(&id).await.unwrap();
    assert_eq!(instance.status, WorkflowStatus::Suspended);

    // The pointer should be in Failed status.
    let failed = instance
        .execution_pointers
        .iter()
        .any(|p| p.status == PointerStatus::Failed);
    assert!(failed, "Expected a failed pointer");

    host.stop().await;
}

#[tokio::test]
async fn terminate_error_behavior_terminates_workflow() {
    let host = make_host();

    let mut def = WorkflowDefinition::new("term-err-wf", 1);
    let mut step0 = WorkflowStep::new(0, std::any::type_name::<AlwaysFailStep>());
    step0.error_behavior = Some(ErrorBehavior::Terminate);
    def.steps = vec![step0];

    host.register_step::<AlwaysFailStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        "term-err-wf",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Terminated);
    assert!(instance.complete_time.is_some());

    host.stop().await;
}
