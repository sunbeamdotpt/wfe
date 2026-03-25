use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use wfe::builder::WorkflowBuilder;
use wfe::models::{ExecutionResult, WorkflowStatus};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CounterData {
    counter: i32,
}

/// A step that simply proceeds to the next step.
#[derive(Default)]
struct IncrementStep;

#[async_trait]
impl StepBody for IncrementStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

#[tokio::test]
async fn linear_three_step_workflow_completes() {
    let def = WorkflowBuilder::<CounterData>::new()
        .start_with::<IncrementStep>()
        .name("Step A")
        .then::<IncrementStep>()
        .name("Step B")
        .then::<IncrementStep>()
        .name("Step C")
        .end_workflow()
        .build("linear-wf", 1);

    // Verify the definition has 3 steps wired correctly.
    assert_eq!(def.steps.len(), 3);
    assert_eq!(def.steps[0].outcomes[0].next_step, 1);
    assert_eq!(def.steps[1].outcomes[0].next_step, 2);

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    host.register_step::<IncrementStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        "linear-wf",
        1,
        serde_json::to_value(CounterData::default()).unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // All 3 steps should have run: 1 initial pointer + 2 chained = 3 total pointers.
    let complete_count = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == wfe::models::PointerStatus::Complete)
        .count();
    assert_eq!(complete_count, 3, "Expected 3 completed execution pointers");

    host.stop().await;
}
