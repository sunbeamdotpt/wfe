use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use wfe::WorkflowHostBuilder;
use wfe::models::{
    ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};

/// A step that waits for "approval" event with key "request-1".
#[derive(Default)]
struct WaitForApprovalStep;

#[async_trait]
impl StepBody for WaitForApprovalStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        if ctx.execution_pointer.event_published {
            return Ok(ExecutionResult::next());
        }
        Ok(ExecutionResult::wait_for_event(
            "approval",
            "request-1",
            Utc::now(),
        ))
    }
}

/// A final step after the event is received.
#[derive(Default)]
struct FinalStep;

#[async_trait]
impl StepBody for FinalStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_event_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("event-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<WaitForApprovalStep>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<FinalStep>());

    def.steps = vec![step0, step1];
    def
}

#[tokio::test]
async fn event_workflow_waits_then_resumes_on_publish() {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    let def = build_event_definition();
    host.register_step::<WaitForApprovalStep>().await;
    host.register_step::<FinalStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let id = host
        .start_workflow("event-wf", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Wait for the workflow to reach WaitingForEvent status.
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("Workflow pointer did not reach WaitingForEvent");
        }
        let instance = host.get_workflow(&id).await.unwrap();
        let waiting = instance
            .execution_pointers
            .iter()
            .any(|p| p.status == PointerStatus::WaitingForEvent);
        if waiting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Verify the workflow is still Runnable but stuck waiting.
    let instance = host.get_workflow(&id).await.unwrap();
    assert_eq!(instance.status, WorkflowStatus::Runnable);

    // Publish the matching event.
    host.publish_event(
        "approval",
        "request-1",
        serde_json::json!({"approved": true}),
    )
    .await
    .unwrap();

    // Wait for workflow to complete.
    let start2 = tokio::time::Instant::now();
    loop {
        if start2.elapsed() > timeout {
            panic!("Workflow did not complete after event was published");
        }
        let instance = host.get_workflow(&id).await.unwrap();
        if instance.status == WorkflowStatus::Complete {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Verify final state.
    let instance = host.get_workflow(&id).await.unwrap();
    assert_eq!(instance.status, WorkflowStatus::Complete);

    // The WaitFor step's pointer should have event_data set.
    let wait_pointer = instance
        .execution_pointers
        .iter()
        .find(|p| p.event_data.is_some());
    assert!(
        wait_pointer.is_some(),
        "Expected event_data to be set on the waiting pointer"
    );

    let event_data = wait_pointer.unwrap().event_data.as_ref().unwrap();
    assert_eq!(event_data, &serde_json::json!({"approved": true}));

    host.stop().await;
}
