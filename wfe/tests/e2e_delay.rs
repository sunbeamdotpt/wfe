use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use wfe::models::{
    ExecutionResult, StepOutcome, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};


/// A step that sleeps for a very short duration (10ms), then proceeds.
/// Tracks whether it has already slept via persistence_data.
#[derive(Default)]
struct ShortDelayStep;

#[async_trait]
impl StepBody for ShortDelayStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let already_slept = ctx
            .persistence_data
            .and_then(|d| d.get("slept"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if already_slept {
            Ok(ExecutionResult::next())
        } else {
            Ok(ExecutionResult::sleep(
                Duration::from_millis(10),
                Some(serde_json::json!({"slept": true})),
            ))
        }
    }
}

/// A step that runs after the delay.
#[derive(Default)]
struct AfterDelayStep;

#[async_trait]
impl StepBody for AfterDelayStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_delay_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("delay-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<ShortDelayStep>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<AfterDelayStep>());

    def.steps = vec![step0, step1];
    def
}

#[tokio::test]
async fn delay_step_completes_after_sleep() {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    let def = build_delay_definition();
    host.register_step::<ShortDelayStep>().await;
    host.register_step::<AfterDelayStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        "delay-wf",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Both steps should have completed.
    let complete_count = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == wfe::models::PointerStatus::Complete)
        .count();
    assert_eq!(complete_count, 2, "Expected both delay step and after-delay step to complete");

    host.stop().await;
}
