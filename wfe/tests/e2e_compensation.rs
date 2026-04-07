use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use wfe::WorkflowHostBuilder;
use wfe::models::{
    ErrorBehavior, ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};

/// Step 1: succeeds normally.
#[derive(Default)]
struct SucceedingStep;

#[async_trait]
impl StepBody for SucceedingStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

/// Step 2: always fails (triggers compensation).
#[derive(Default)]
struct FailingStep;

#[async_trait]
impl StepBody for FailingStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Err(wfe_core::WfeError::StepExecution("Step2 failed".into()))
    }
}

/// Compensation step for Step 1.
#[derive(Default)]
struct CompensateStep1;

#[async_trait]
impl StepBody for CompensateStep1 {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_compensation_definition() -> WorkflowDefinition {
    // Step 0: SucceedingStep -> Step 1 (has compensation at step 2)
    // Step 1: FailingStep (error_behavior = Compensate, compensation_step_id = 3)
    // Step 2: CompensateStep1 (compensation for step 0)
    // Step 3: CompensateStep1 (compensation for step 1 -- triggered on failure)
    //
    // When FailingStep fails with Compensate behavior, the error handler creates
    // a new pointer to the compensation step.
    let mut def = WorkflowDefinition::new("comp-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<SucceedingStep>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });

    let mut step1 = WorkflowStep::new(1, std::any::type_name::<FailingStep>());
    step1.error_behavior = Some(ErrorBehavior::Compensate);
    step1.compensation_step_id = Some(2);

    let step2 = WorkflowStep::new(2, std::any::type_name::<CompensateStep1>());

    def.steps = vec![step0, step1, step2];
    def
}

#[tokio::test]
async fn compensation_step_runs_on_failure() {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    let def = build_compensation_definition();
    host.register_step::<SucceedingStep>().await;
    host.register_step::<FailingStep>().await;
    host.register_step::<CompensateStep1>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let id = host
        .start_workflow("comp-wf", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Wait for the workflow to reach a terminal state.
    // With Compensate behavior, the failing step gets Failed status and a compensation
    // pointer is created. The workflow should eventually complete since all pointers
    // reach terminal states (Complete or Failed).
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    let mut final_instance = None;
    loop {
        if start.elapsed() > timeout {
            break;
        }
        let instance = host.get_workflow(&id).await.unwrap();
        let all_terminal = !instance.execution_pointers.is_empty()
            && instance.execution_pointers.iter().all(|p| {
                matches!(
                    p.status,
                    PointerStatus::Complete | PointerStatus::Failed | PointerStatus::Compensated
                )
            });
        if all_terminal || instance.status != wfe::models::WorkflowStatus::Runnable {
            final_instance = Some(instance);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let instance = final_instance.expect("Workflow should have reached terminal state");

    // The FailingStep pointer should be Failed.
    let failed_pointer = instance
        .execution_pointers
        .iter()
        .find(|p| p.step_id == 1 && p.status == PointerStatus::Failed);
    assert!(
        failed_pointer.is_some(),
        "Expected FailingStep pointer to be in Failed status"
    );

    // The compensation step (step 2) should have been created and completed.
    let comp_pointer = instance
        .execution_pointers
        .iter()
        .find(|p| p.step_id == 2 && p.status == PointerStatus::Complete);
    assert!(
        comp_pointer.is_some(),
        "Expected compensation step to have run and completed"
    );

    host.stop().await;
}
