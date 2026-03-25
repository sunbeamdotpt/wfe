use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use wfe::models::{
    ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};


/// Version 1 uses a single step.
#[derive(Default)]
struct V1Step;

#[async_trait]
impl StepBody for V1Step {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

/// Version 2 uses two steps.
#[derive(Default)]
struct V2StepA;

#[async_trait]
impl StepBody for V2StepA {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct V2StepB;

#[async_trait]
impl StepBody for V2StepB {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_v1_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("versioned-wf", 1);
    let step0 = WorkflowStep::new(0, std::any::type_name::<V1Step>());
    def.steps = vec![step0];
    def
}

fn build_v2_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("versioned-wf", 2);
    let mut step0 = WorkflowStep::new(0, std::any::type_name::<V2StepA>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<V2StepB>());
    def.steps = vec![step0, step1];
    def
}

#[tokio::test]
async fn version_1_uses_v1_definition() {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    host.register_step::<V1Step>().await;
    host.register_step::<V2StepA>().await;
    host.register_step::<V2StepB>().await;

    host.register_workflow_definition(build_v1_definition()).await;
    host.register_workflow_definition(build_v2_definition()).await;

    host.start().await.unwrap();

    // Start with version 1.
    let instance = run_workflow_sync(
        &host,
        "versioned-wf",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.version, 1);

    // V1 has 1 step, so 1 pointer.
    let complete_count = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete)
        .count();
    assert_eq!(complete_count, 1, "V1 should have 1 completed pointer");

    host.stop().await;
}

#[tokio::test]
async fn version_2_uses_v2_definition() {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    host.register_step::<V1Step>().await;
    host.register_step::<V2StepA>().await;
    host.register_step::<V2StepB>().await;

    host.register_workflow_definition(build_v1_definition()).await;
    host.register_workflow_definition(build_v2_definition()).await;

    host.start().await.unwrap();

    // Start with version 2.
    let instance = run_workflow_sync(
        &host,
        "versioned-wf",
        2,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.version, 2);

    // V2 has 2 steps, so 2 pointers.
    let complete_count = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete)
        .count();
    assert_eq!(complete_count, 2, "V2 should have 2 completed pointers");

    host.stop().await;
}

#[tokio::test]
async fn both_versions_coexist_and_run_independently() {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    host.register_step::<V1Step>().await;
    host.register_step::<V2StepA>().await;
    host.register_step::<V2StepB>().await;

    host.register_workflow_definition(build_v1_definition()).await;
    host.register_workflow_definition(build_v2_definition()).await;

    host.start().await.unwrap();

    // Start both versions concurrently.
    let id_v1 = host
        .start_workflow("versioned-wf", 1, serde_json::json!({}))
        .await
        .unwrap();
    let id_v2 = host
        .start_workflow("versioned-wf", 2, serde_json::json!({}))
        .await
        .unwrap();

    // Wait for both to complete.
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("Workflows did not complete within timeout");
        }
        let inst_v1 = host.get_workflow(&id_v1).await.unwrap();
        let inst_v2 = host.get_workflow(&id_v2).await.unwrap();
        if inst_v1.status == WorkflowStatus::Complete
            && inst_v2.status == WorkflowStatus::Complete
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let inst_v1 = host.get_workflow(&id_v1).await.unwrap();
    let inst_v2 = host.get_workflow(&id_v2).await.unwrap();

    assert_eq!(inst_v1.version, 1);
    assert_eq!(inst_v2.version, 2);

    let v1_pointers = inst_v1
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete)
        .count();
    let v2_pointers = inst_v2
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete)
        .count();

    assert_eq!(v1_pointers, 1, "V1 should have 1 completed pointer");
    assert_eq!(v2_pointers, 2, "V2 should have 2 completed pointers");

    host.stop().await;
}
