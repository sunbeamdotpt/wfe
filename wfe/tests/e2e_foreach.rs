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
struct ForeachData {
    items: Vec<String>,
}

/// A ForEachStep-like step that reads its collection from workflow data.
#[derive(Default)]
struct DataDrivenForEachStep;

#[async_trait]
impl StepBody for DataDrivenForEachStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let items: Vec<serde_json::Value> = ctx
            .workflow
            .data
            .get("items")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if items.is_empty() {
            return Ok(ExecutionResult::next());
        }

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
            // Branch with all items (parallel execution).
            Ok(ExecutionResult::branch(
                items,
                Some(json!({"children_active": true})),
            ))
        }
    }
}

/// A child step that processes each item.
#[derive(Default)]
struct ProcessItemStep;

#[async_trait]
impl StepBody for ProcessItemStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

fn build_foreach_definition() -> WorkflowDefinition {
    // Step 0: DataDrivenForEachStep (container, children: [1])
    // Step 1: ProcessItemStep (child of 0)
    let mut def = WorkflowDefinition::new("foreach-wf", 1);

    let mut step0 = WorkflowStep::new(0, std::any::type_name::<DataDrivenForEachStep>());
    step0.children = vec![1];

    let step1 = WorkflowStep::new(1, std::any::type_name::<ProcessItemStep>());

    def.steps = vec![step0, step1];
    def
}

#[tokio::test]
async fn foreach_processes_all_items() {
    let def = build_foreach_definition();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build().unwrap();

    host.register_step::<DataDrivenForEachStep>().await;
    host.register_step::<ProcessItemStep>().await;
    host.register_workflow_definition(def).await;
    host.start().await.unwrap();

    let data = ForeachData {
        items: vec!["apple".into(), "banana".into(), "cherry".into()],
    };

    let instance = run_workflow_sync(
        &host,
        "foreach-wf",
        1,
        serde_json::to_value(data).unwrap(),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // 3 items should produce 3 child pointers (branched from the ForEach container).
    let child_runs = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete && !p.scope.is_empty())
        .count();
    assert_eq!(
        child_runs, 3,
        "Expected 3 child step executions for 3 items"
    );

    // Verify each child received a context_item.
    let items_seen: Vec<&serde_json::Value> = instance
        .execution_pointers
        .iter()
        .filter(|p| !p.scope.is_empty() && p.context_item.is_some())
        .filter_map(|p| p.context_item.as_ref())
        .collect();
    assert_eq!(items_seen.len(), 3);

    host.stop().await;
}
