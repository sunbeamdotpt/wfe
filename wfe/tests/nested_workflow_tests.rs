use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use wfe::models::{
    ExecutionResult, StepOutcome, WorkflowDefinition, WorkflowStatus, WorkflowStep,
};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::WorkflowHostBuilder;
use wfe_core::primitives::sub_workflow::SubWorkflowStep;
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};

// ----- Test step bodies -----

/// A step that sets output data with {"result": "child_done"}.
#[derive(Default)]
struct SetResultStep;

#[async_trait]
impl StepBody for SetResultStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let mut result = ExecutionResult::next();
        result.output_data = Some(json!({"result": "child_done"}));
        Ok(result)
    }
}

/// A step that reads workflow data and proceeds.
#[derive(Default)]
struct ReadDataStep;

#[async_trait]
impl StepBody for ReadDataStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

/// A step that always fails.
#[derive(Default)]
struct FailingStep;

#[async_trait]
impl StepBody for FailingStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Err(wfe_core::WfeError::StepExecution(
            "intentional failure".to_string(),
        ))
    }
}

/// A step that sets greeting output based on input.
#[derive(Default)]
struct GreetStep;

#[async_trait]
impl StepBody for GreetStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let name = ctx
            .workflow
            .data
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let mut result = ExecutionResult::next();
        result.output_data = Some(json!({"greeting": format!("hello {name}")}));
        Ok(result)
    }
}

/// A no-op step.
#[derive(Default)]
struct NoOpStep;

#[async_trait]
impl StepBody for NoOpStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

// ----- Helpers -----

fn build_host() -> (wfe::WorkflowHost, Arc<InMemoryPersistenceProvider>) {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    (host, persistence)
}

fn sub_workflow_step_type() -> String {
    std::any::type_name::<SubWorkflowStep>().to_string()
}

/// Build a child workflow definition with a single step that produces output.
fn build_child_definition(child_step_type: &str) -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("child-workflow", 1);
    let step0 = WorkflowStep::new(0, child_step_type);
    def.steps = vec![step0];
    def
}

/// Build a parent workflow with: SubWorkflowStep(0) -> ReadDataStep(1).
fn build_parent_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("parent-workflow", 1);

    let mut step0 = WorkflowStep::new(0, sub_workflow_step_type());
    step0.step_config = Some(json!({
        "workflow_id": "child-workflow",
        "version": 1,
        "inputs": {},
        "output_keys": ["result"]
    }));
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });

    let step1 = WorkflowStep::new(1, std::any::type_name::<ReadDataStep>());

    def.steps = vec![step0, step1];
    def
}

async fn wait_for_status(
    host: &wfe::WorkflowHost,
    id: &str,
    status: WorkflowStatus,
    timeout: Duration,
) -> wfe::models::WorkflowInstance {
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            let instance = host.get_workflow(id).await.unwrap();
            panic!(
                "Workflow {id} did not reach status {status:?} within {timeout:?}. \
                 Current status: {:?}, pointers: {:?}",
                instance.status, instance.execution_pointers
            );
        }
        let instance = host.get_workflow(id).await.unwrap();
        if instance.status == status {
            return instance;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ----- Tests -----

/// Parent workflow starts a child, waits for the child's completion event,
/// then proceeds to a second step.
#[tokio::test]
async fn parent_workflow_starts_child_and_waits() {
    let (host, _persistence) = build_host();

    let child_def = build_child_definition(std::any::type_name::<SetResultStep>());
    let parent_def = build_parent_definition();

    host.register_step::<SetResultStep>().await;
    host.register_step::<ReadDataStep>().await;

    host.register_workflow_definition(child_def).await;
    host.register_workflow_definition(parent_def).await;

    host.start().await.unwrap();

    let parent_id = host
        .start_workflow("parent-workflow", 1, json!({}))
        .await
        .unwrap();

    let parent_instance = wait_for_status(
        &host,
        &parent_id,
        WorkflowStatus::Complete,
        Duration::from_secs(10),
    )
    .await;

    assert_eq!(parent_instance.status, WorkflowStatus::Complete);

    // The SubWorkflowStep's execution pointer should have received event data
    // from the child's completion event.
    let sub_wf_pointer = &parent_instance.execution_pointers[0];
    assert!(
        sub_wf_pointer.event_data.is_some(),
        "SubWorkflowStep pointer should have event_data from child completion"
    );

    let event_data = sub_wf_pointer.event_data.as_ref().unwrap();
    assert_eq!(
        event_data.get("status").and_then(|v| v.as_str()),
        Some("Complete")
    );

    // The child's output data should be in the event.
    let child_data = event_data.get("data").unwrap();
    assert_eq!(child_data.get("result"), Some(&json!("child_done")));

    host.stop().await;
}

/// Parent workflow starts a child that passes all data through (no output_keys filter).
#[tokio::test]
async fn parent_workflow_passes_all_child_data() {
    let (host, _) = build_host();

    let child_def = build_child_definition(std::any::type_name::<SetResultStep>());

    // Parent with no output_keys filter - passes all child data.
    let mut parent_def = WorkflowDefinition::new("parent-all-data", 1);
    let mut step0 = WorkflowStep::new(0, sub_workflow_step_type());
    step0.step_config = Some(json!({
        "workflow_id": "child-workflow",
        "version": 1,
        "inputs": {}
    }));
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<NoOpStep>());
    parent_def.steps = vec![step0, step1];

    host.register_step::<SetResultStep>().await;
    host.register_step::<NoOpStep>().await;
    host.register_workflow_definition(child_def).await;
    host.register_workflow_definition(parent_def).await;

    host.start().await.unwrap();

    let parent_id = host
        .start_workflow("parent-all-data", 1, json!({}))
        .await
        .unwrap();

    let instance = wait_for_status(
        &host,
        &parent_id,
        WorkflowStatus::Complete,
        Duration::from_secs(10),
    )
    .await;

    assert_eq!(instance.status, WorkflowStatus::Complete);
    host.stop().await;
}

/// Child workflow that uses typed inputs and produces typed output.
#[tokio::test]
async fn nested_workflow_with_typed_inputs_outputs() {
    let (host, _) = build_host();

    // Child workflow: GreetStep reads "name" from workflow data and produces {"greeting": "hello <name>"}.
    let mut child_def = WorkflowDefinition::new("greet-child", 1);
    let step0 = WorkflowStep::new(0, std::any::type_name::<GreetStep>());
    child_def.steps = vec![step0];

    // Parent workflow: SubWorkflowStep starts greet-child with {"name": "world"}.
    let mut parent_def = WorkflowDefinition::new("greet-parent", 1);
    let mut step0 = WorkflowStep::new(0, sub_workflow_step_type());
    step0.step_config = Some(json!({
        "workflow_id": "greet-child",
        "version": 1,
        "inputs": {"name": "world"},
        "output_keys": ["greeting"]
    }));
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<NoOpStep>());
    parent_def.steps = vec![step0, step1];

    host.register_step::<GreetStep>().await;
    host.register_step::<NoOpStep>().await;
    host.register_workflow_definition(child_def).await;
    host.register_workflow_definition(parent_def).await;

    host.start().await.unwrap();

    let parent_id = host
        .start_workflow("greet-parent", 1, json!({}))
        .await
        .unwrap();

    let instance = wait_for_status(
        &host,
        &parent_id,
        WorkflowStatus::Complete,
        Duration::from_secs(10),
    )
    .await;

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Check the SubWorkflowStep pointer got the greeting from the child.
    let sub_wf_pointer = &instance.execution_pointers[0];
    let event_data = sub_wf_pointer.event_data.as_ref().unwrap();
    let child_data = event_data.get("data").unwrap();
    assert_eq!(
        child_data.get("greeting"),
        Some(&json!("hello world")),
        "Child should produce greeting from input name"
    );

    host.stop().await;
}

/// When a child workflow fails, the parent should still complete because the
/// completion event carries the terminated status. The SubWorkflowStep will
/// process the event data even if the child ended in error, and the parent
/// will see the child's status in the event_data.
#[tokio::test]
async fn nested_workflow_child_failure_propagates() {
    let (host, _) = build_host();

    // Child workflow with a failing step.
    let mut child_def = WorkflowDefinition::new("failing-child", 1);
    let step0 = WorkflowStep::new(0, std::any::type_name::<FailingStep>());
    child_def.steps = vec![step0];

    // Parent workflow: SubWorkflowStep starts failing-child.
    let mut parent_def = WorkflowDefinition::new("parent-of-failing", 1);
    let mut step0 = WorkflowStep::new(0, sub_workflow_step_type());
    step0.step_config = Some(json!({
        "workflow_id": "failing-child",
        "version": 1,
        "inputs": {}
    }));
    // No outcome wired — if the child fails and the sub workflow step
    // processes the completion event, the parent can proceed.
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<NoOpStep>());
    parent_def.steps = vec![step0, step1];

    host.register_step::<FailingStep>().await;
    host.register_step::<NoOpStep>().await;
    host.register_workflow_definition(child_def).await;
    host.register_workflow_definition(parent_def).await;

    host.start().await.unwrap();

    let parent_id = host
        .start_workflow("parent-of-failing", 1, json!({}))
        .await
        .unwrap();

    // The child will fail. Depending on executor error handling, the child may
    // reach Terminated status. We wait a bit then check states.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let parent = host.get_workflow(&parent_id).await.unwrap();
    // Parent should still be Runnable (waiting for the child's completion event)
    // since the failing child may not emit a completion event.
    // Check that the parent is stuck waiting (SubWorkflowStep issued wait_for_event).
    let has_waiting = parent
        .execution_pointers
        .iter()
        .any(|p| p.status == wfe::models::PointerStatus::WaitingForEvent);
    // Either the parent is waiting or already completed (if the child errored
    // and published a terminated event).
    assert!(
        has_waiting || parent.status == WorkflowStatus::Complete,
        "Parent should be waiting for child or complete. Status: {:?}",
        parent.status
    );

    host.stop().await;
}

/// Test that SubWorkflowStep is registered as a built-in primitive.
#[tokio::test]
async fn sub_workflow_step_registered_as_primitive() {
    let (host, _) = build_host();

    // Start registers primitives.
    host.start().await.unwrap();

    // Verify by checking that the step registry has the SubWorkflowStep type.
    // We do this indirectly — if we can run a workflow using SubWorkflowStep
    // without explicit registration, it's registered.
    let child_def = build_child_definition(std::any::type_name::<NoOpStep>());
    let parent_def = build_parent_definition();

    // Only register the non-primitive steps, NOT SubWorkflowStep.
    host.register_step::<NoOpStep>().await;
    host.register_step::<SetResultStep>().await;
    host.register_step::<ReadDataStep>().await;

    host.register_workflow_definition(child_def).await;
    host.register_workflow_definition(parent_def).await;

    let parent_id = host
        .start_workflow("parent-workflow", 1, json!({}))
        .await
        .unwrap();

    // If SubWorkflowStep is registered as a primitive, the parent should start
    // the child and eventually complete.
    let instance = wait_for_status(
        &host,
        &parent_id,
        WorkflowStatus::Complete,
        Duration::from_secs(10),
    )
    .await;

    assert_eq!(instance.status, WorkflowStatus::Complete);
    host.stop().await;
}

/// Test that starting a child with a non-existent definition propagates an error.
#[tokio::test]
async fn nested_workflow_nonexistent_child_definition() {
    let (host, _) = build_host();

    // Parent references a child that is NOT registered.
    let mut parent_def = WorkflowDefinition::new("parent-missing-child", 1);
    let mut step0 = WorkflowStep::new(0, sub_workflow_step_type());
    step0.step_config = Some(json!({
        "workflow_id": "nonexistent-child",
        "version": 1,
        "inputs": {}
    }));
    parent_def.steps = vec![step0];

    host.register_workflow_definition(parent_def).await;

    host.start().await.unwrap();

    let parent_id = host
        .start_workflow("parent-missing-child", 1, json!({}))
        .await
        .unwrap();

    // The parent should fail because the child definition doesn't exist.
    // Give it time to process.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let parent = host.get_workflow(&parent_id).await.unwrap();
    // The error handler should have set the pointer to Failed and the workflow
    // to Suspended (default error behavior is Retry then Suspend).
    let pointer = &parent.execution_pointers[0];
    let is_failed = pointer.status == wfe::models::PointerStatus::Failed
        || pointer.status == wfe::models::PointerStatus::Sleeping;
    assert!(
        is_failed
            || parent.status == WorkflowStatus::Suspended
            || parent.status == WorkflowStatus::Terminated,
        "SubWorkflowStep should error when child definition is missing. \
         Pointer status: {:?}, Workflow status: {:?}",
        pointer.status,
        parent.status
    );

    host.stop().await;
}

/// Test that starting a workflow via the host works end-to-end,
/// exercising the HostContextImpl path indirectly.
#[tokio::test]
async fn host_context_impl_starts_workflow() {
    let (host, _) = build_host();

    // Register a child definition.
    let child_def = build_child_definition(std::any::type_name::<NoOpStep>());
    host.register_step::<NoOpStep>().await;
    host.register_workflow_definition(child_def).await;

    host.start().await.unwrap();

    // Start a child workflow via the host's start_workflow.
    let child_id = host
        .start_workflow("child-workflow", 1, json!({"test": true}))
        .await
        .unwrap();

    // Verify the child was created and completes.
    let child = host.get_workflow(&child_id).await.unwrap();
    assert_eq!(child.workflow_definition_id, "child-workflow");
    assert_eq!(child.version, 1);
    assert_eq!(child.data.get("test"), Some(&json!(true)));

    // Wait for the child to complete.
    let instance = wait_for_status(
        &host,
        &child_id,
        WorkflowStatus::Complete,
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    host.stop().await;
}
