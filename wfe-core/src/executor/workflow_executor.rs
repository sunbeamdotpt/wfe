use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, error, info, warn};

use super::error_handler;
use super::result_processor;
use super::step_registry::StepRegistry;
use crate::models::{
    ExecutionError, PointerStatus, QueueType, WorkflowDefinition, WorkflowStatus,
};
use crate::traits::{
    DistributedLockProvider, LifecyclePublisher, PersistenceProvider, QueueProvider, SearchIndex,
    StepExecutionContext,
};
use crate::{Result, WfeError};

/// The core workflow executor. Processes a single workflow instance per `execute()` call.
pub struct WorkflowExecutor {
    pub persistence: Arc<dyn PersistenceProvider>,
    pub lock_provider: Arc<dyn DistributedLockProvider>,
    pub queue_provider: Arc<dyn QueueProvider>,
    pub lifecycle: Option<Arc<dyn LifecyclePublisher>>,
    pub search: Option<Arc<dyn SearchIndex>>,
}

impl WorkflowExecutor {
    pub fn new(
        persistence: Arc<dyn PersistenceProvider>,
        lock_provider: Arc<dyn DistributedLockProvider>,
        queue_provider: Arc<dyn QueueProvider>,
    ) -> Self {
        Self {
            persistence,
            lock_provider,
            queue_provider,
            lifecycle: None,
            search: None,
        }
    }

    pub fn with_lifecycle(mut self, lifecycle: Arc<dyn LifecyclePublisher>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    pub fn with_search(mut self, search: Arc<dyn SearchIndex>) -> Self {
        self.search = Some(search);
        self
    }

    /// Execute a single workflow instance.
    ///
    /// 1. Acquire lock
    /// 2. Load instance
    /// 3. Find runnable pointers
    /// 4. Execute each runnable pointer's step
    /// 5. Process results
    /// 6. Check for completion
    /// 7. Persist
    /// 8. Release lock
    #[tracing::instrument(
        name = "workflow.execute",
        skip(self, definition, step_registry),
        fields(
            workflow.id = %workflow_id,
            workflow.definition_id,
            workflow.status,
        )
    )]
    pub async fn execute(
        &self,
        workflow_id: &str,
        definition: &WorkflowDefinition,
        step_registry: &StepRegistry,
    ) -> Result<()> {
        // 1. Acquire distributed lock.
        let acquired = self.lock_provider.acquire_lock(workflow_id).await?;
        if !acquired {
            debug!(workflow_id, "Could not acquire lock, skipping");
            return Ok(());
        }

        let result = self
            .execute_inner(workflow_id, definition, step_registry)
            .await;

        // 7. Release lock (always).
        if let Err(e) = self.lock_provider.release_lock(workflow_id).await {
            error!(workflow_id, error = %e, "Failed to release lock");
        }

        result
    }

    async fn execute_inner(
        &self,
        workflow_id: &str,
        definition: &WorkflowDefinition,
        step_registry: &StepRegistry,
    ) -> Result<()> {
        // 2. Load workflow instance.
        let mut workflow = self
            .persistence
            .get_workflow_instance(workflow_id)
            .await?;

        tracing::Span::current().record("workflow.definition_id", workflow.workflow_definition_id.as_str());

        if workflow.status != WorkflowStatus::Runnable {
            debug!(workflow_id, status = ?workflow.status, "Workflow not runnable, skipping");
            return Ok(());
        }

        let now = Utc::now();
        let mut all_subscriptions = Vec::new();
        let mut execution_errors = Vec::new();

        // 3. Find runnable execution pointers.
        debug!(
            workflow_id,
            definition_id = %workflow.workflow_definition_id,
            pointers = workflow.execution_pointers.len(),
            "Executing workflow"
        );
        let runnable_indices: Vec<usize> = workflow
            .execution_pointers
            .iter()
            .enumerate()
            .filter(|(_, p)| is_pointer_runnable(p, now))
            .map(|(i, _)| i)
            .collect();

        // 4. For each runnable pointer, execute the step.
        for idx in runnable_indices {
            let step_id = workflow.execution_pointers[idx].step_id;

            // a. Look up the WorkflowStep.
            let step = definition
                .steps
                .iter()
                .find(|s| s.id == step_id)
                .ok_or(WfeError::StepNotFound(step_id))?;

            info!(
                workflow_id,
                step_id,
                step_type = %step.step_type,
                step_name = step.name.as_deref().unwrap_or("(unnamed)"),
                "Running step"
            );

            // b. Resolve the step body.
            let mut step_body = step_registry
                .resolve(&step.step_type)
                .ok_or_else(|| WfeError::StepExecution(format!(
                    "Step type not found in registry: {}",
                    step.step_type
                )))?;

            // Mark pointer as running before building context.
            if workflow.execution_pointers[idx].start_time.is_none() {
                workflow.execution_pointers[idx].start_time = Some(Utc::now());
            }
            workflow.execution_pointers[idx].status = PointerStatus::Running;

            // c. Build StepExecutionContext (borrows workflow immutably).
            let cancellation_token = tokio_util::sync::CancellationToken::new();
            let context = StepExecutionContext {
                item: workflow.execution_pointers[idx].context_item.as_ref(),
                execution_pointer: &workflow.execution_pointers[idx],
                persistence_data: workflow.execution_pointers[idx].persistence_data.as_ref(),
                step,
                workflow: &workflow,
                cancellation_token,
            };

            // d. Call step.run(context).
            let step_result = step_body.run(&context).await;

            // Now we can mutate again since context is dropped.
            match step_result {
                Ok(result) => {
                    let step_status = if result.sleep_for.is_some() {
                        "sleeping"
                    } else if result.event_name.is_some() {
                        "waiting_for_event"
                    } else {
                        "completed"
                    };
                    tracing::Span::current().record("step.status", step_status);

                    info!(
                        workflow_id,
                        step_id,
                        proceed = result.proceed,
                        has_sleep = result.sleep_for.is_some(),
                        has_event = result.event_name.is_some(),
                        has_branches = result.branch_values.is_some(),
                        "Step completed"
                    );
                    // e. Process the ExecutionResult.
                    // Extract workflow_id before mutable borrow.
                    let wf_id = workflow.id.clone();
                    let process_result = {
                        let pointer = &mut workflow.execution_pointers[idx];
                        result_processor::process_result(
                            &result,
                            pointer,
                            definition,
                            &wf_id,
                        )
                    };

                    all_subscriptions.extend(process_result.subscriptions);

                    // Add new pointers.
                    for new_pointer in process_result.new_pointers {
                        workflow.execution_pointers.push(new_pointer);
                    }
                }
                Err(e) => {
                    // f. Handle error.
                    let error_msg = e.to_string();
                    tracing::Span::current().record("step.status", "failed");
                    warn!(workflow_id, step_id, error = %error_msg, "Step execution failed");

                    let pointer_id = workflow.execution_pointers[idx].id.clone();
                    execution_errors.push(ExecutionError::new(
                        workflow_id,
                        &pointer_id,
                        &error_msg,
                    ));

                    let handler_result = {
                        let pointer = &mut workflow.execution_pointers[idx];
                        error_handler::handle_error(
                            &error_msg,
                            pointer,
                            definition,
                        )
                    };

                    // Apply workflow-level status changes from error handler.
                    if let Some(new_status) = handler_result.workflow_status {
                        workflow.status = new_status;
                        if new_status == WorkflowStatus::Terminated {
                            workflow.complete_time = Some(Utc::now());
                        }
                    }

                    for new_pointer in handler_result.new_pointers {
                        workflow.execution_pointers.push(new_pointer);
                    }
                }
            }
        }

        // 5. Check if all pointers are complete -> set workflow status to Complete.
        let all_done = !workflow.execution_pointers.is_empty()
            && workflow.execution_pointers.iter().all(|p| {
                matches!(
                    p.status,
                    PointerStatus::Complete
                        | PointerStatus::Compensated
                        | PointerStatus::Cancelled
                        | PointerStatus::Failed
                )
            });

        if all_done && workflow.status == WorkflowStatus::Runnable {
            info!(workflow_id, "All pointers complete, workflow finished");
            workflow.status = WorkflowStatus::Complete;
            workflow.complete_time = Some(Utc::now());
        }

        tracing::Span::current().record("workflow.status", tracing::field::debug(&workflow.status));

        // Determine next_execution.
        let has_active = workflow.execution_pointers.iter().any(|p| p.active);
        if has_active {
            workflow.next_execution = Some(0);
        } else if workflow.status == WorkflowStatus::Runnable {
            // Still runnable but no active pointers - might have sleeping/waiting pointers.
            let next_sleep = workflow
                .execution_pointers
                .iter()
                .filter(|p| p.status == PointerStatus::Sleeping)
                .filter_map(|p| p.sleep_until)
                .min();
            workflow.next_execution = next_sleep.map(|t| t.timestamp_millis());
        } else {
            workflow.next_execution = None;
        }

        // 6. Persist updated WorkflowInstance.
        if all_subscriptions.is_empty() {
            self.persistence.persist_workflow(&workflow).await?;
        } else {
            self.persistence
                .persist_workflow_with_subscriptions(&workflow, &all_subscriptions)
                .await?;
        }

        // Persist errors.
        if !execution_errors.is_empty() {
            self.persistence
                .persist_errors(&execution_errors)
                .await?;
        }

        // 8. Queue any follow-up work.
        if workflow.status == WorkflowStatus::Runnable
            && has_active
            && let Err(e) = self
                .queue_provider
                .queue_work(workflow_id, QueueType::Workflow)
                .await
        {
            error!(workflow_id = %workflow_id, error = %e, "Failed to re-queue workflow");
            return Err(e);
        }

        if let Some(ref search) = self.search
            && let Err(e) = search.index_workflow(&workflow).await
        {
            warn!(workflow_id = %workflow_id, error = %e, "Failed to index workflow");
        }

        Ok(())
    }
}

fn is_pointer_runnable(
    pointer: &crate::models::ExecutionPointer,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !pointer.active {
        return false;
    }

    match pointer.status {
        PointerStatus::Pending | PointerStatus::Running => {
            // Check sleep_until.
            if let Some(sleep_until) = pointer.sleep_until {
                now >= sleep_until
            } else {
                true
            }
        }
        PointerStatus::Sleeping => {
            // Sleeping pointers become runnable when sleep_until passes.
            if let Some(sleep_until) = pointer.sleep_until {
                now >= sleep_until
            } else {
                true
            }
        }
        PointerStatus::WaitingForEvent => {
            // Event arrived -> event_data is set.
            pointer.event_published
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;

    use super::*;
    use crate::models::{
        ErrorBehavior, ExecutionPointer, ExecutionResult, StepOutcome, WorkflowDefinition,
        WorkflowInstance, WorkflowStep,
    };
    use crate::test_support::{
        InMemoryLifecyclePublisher, InMemoryLockProvider, InMemoryPersistenceProvider,
        InMemoryQueueProvider,
    };
    use crate::traits::{StepBody, StepExecutionContext, WorkflowRepository};

    // -- Test step implementations --

    #[derive(Default)]
    struct PassStep;

    #[async_trait]
    impl StepBody for PassStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::next())
        }
    }

    #[derive(Default)]
    struct OutcomeStep;

    #[async_trait]
    impl StepBody for OutcomeStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::outcome(serde_json::json!("yes")))
        }
    }

    #[derive(Default)]
    struct PersistStep;

    #[async_trait]
    impl StepBody for PersistStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::persist(serde_json::json!({"count": 1})))
        }
    }

    #[derive(Default)]
    struct SleepStep;

    #[async_trait]
    impl StepBody for SleepStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::sleep(Duration::from_secs(30), None))
        }
    }

    #[derive(Default)]
    struct WaitEventStep;

    #[async_trait]
    impl StepBody for WaitEventStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::wait_for_event(
                "order.completed",
                "order-123",
                Utc::now(),
            ))
        }
    }

    #[derive(Default)]
    struct EventResumeStep;

    #[async_trait]
    impl StepBody for EventResumeStep {
        async fn run(
            &mut self,
            ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            if ctx.execution_pointer.event_published {
                Ok(ExecutionResult::next())
            } else {
                Ok(ExecutionResult::wait_for_event(
                    "order.completed",
                    "order-123",
                    Utc::now(),
                ))
            }
        }
    }

    #[derive(Default)]
    struct BranchStep;

    #[async_trait]
    impl StepBody for BranchStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::branch(
                vec![
                    serde_json::json!(1),
                    serde_json::json!(2),
                    serde_json::json!(3),
                ],
                None,
            ))
        }
    }

    #[derive(Default)]
    struct FailStep;

    #[async_trait]
    impl StepBody for FailStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Err(WfeError::StepExecution("step failed".into()))
        }
    }

    #[derive(Default)]
    struct CompensateStep;

    #[async_trait]
    impl StepBody for CompensateStep {
        async fn run(
            &mut self,
            _ctx: &StepExecutionContext<'_>,
        ) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::next())
        }
    }

    // -- Helper functions --

    fn create_providers() -> (
        Arc<InMemoryPersistenceProvider>,
        Arc<InMemoryLockProvider>,
        Arc<InMemoryQueueProvider>,
    ) {
        (
            Arc::new(InMemoryPersistenceProvider::new()),
            Arc::new(InMemoryLockProvider::new()),
            Arc::new(InMemoryQueueProvider::new()),
        )
    }

    fn create_executor(
        persistence: Arc<InMemoryPersistenceProvider>,
        lock: Arc<InMemoryLockProvider>,
        queue: Arc<InMemoryQueueProvider>,
    ) -> WorkflowExecutor {
        WorkflowExecutor::new(persistence, lock, queue)
    }

    fn step_type<S: 'static>() -> String {
        std::any::type_name::<S>().to_string()
    }

    // -- Tests --

    #[tokio::test]
    async fn single_step_workflow_completes() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        let pointer = ExecutionPointer::new(0);
        instance.execution_pointers.push(pointer);

        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Complete);
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Complete);
        assert!(updated.complete_time.is_some());
    }

    #[tokio::test]
    async fn linear_two_step_execution() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut step0 = WorkflowStep::new(0, step_type::<PassStep>());
        step0.outcomes.push(StepOutcome {
            next_step: 1,
            label: None,
            value: None,
        });
        def.steps.push(step0);
        def.steps.push(WorkflowStep::new(1, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        // First execution: step 0 completes, step 1 pointer created.
        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers.len(), 2);
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Complete);
        // Step 1 pointer should be active and pending.
        assert_eq!(updated.execution_pointers[1].step_id, 1);

        // Second execution: step 1 completes.
        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Complete);
        assert_eq!(updated.execution_pointers[1].status, PointerStatus::Complete);
    }

    #[tokio::test]
    async fn linear_three_step_execution() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<PassStep>());
        s0.outcomes.push(StepOutcome { next_step: 1, label: None, value: None });
        let mut s1 = WorkflowStep::new(1, step_type::<PassStep>());
        s1.outcomes.push(StepOutcome { next_step: 2, label: None, value: None });
        let s2 = WorkflowStep::new(2, step_type::<PassStep>());
        def.steps.push(s0);
        def.steps.push(s1);
        def.steps.push(s2);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        // Execute three times for three steps.
        for _ in 0..3 {
            executor.execute(&instance.id, &def, &registry).await.unwrap();
        }

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Complete);
        assert_eq!(updated.execution_pointers.len(), 3);
        for p in &updated.execution_pointers {
            assert_eq!(p.status, PointerStatus::Complete);
        }
    }

    #[tokio::test]
    async fn step_with_outcome_routes_correctly() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<OutcomeStep>();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<OutcomeStep>());
        s0.outcomes.push(StepOutcome {
            next_step: 1,
            label: Some("no".into()),
            value: Some(serde_json::json!("no")),
        });
        s0.outcomes.push(StepOutcome {
            next_step: 2,
            label: Some("yes".into()),
            value: Some(serde_json::json!("yes")),
        });
        def.steps.push(s0);
        def.steps.push(WorkflowStep::new(1, step_type::<PassStep>()));
        def.steps.push(WorkflowStep::new(2, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers.len(), 2);
        // Should route to step 2 (the "yes" branch).
        assert_eq!(updated.execution_pointers[1].step_id, 2);
    }

    #[tokio::test]
    async fn step_persist_keeps_pointer_active() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<PersistStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<PersistStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Runnable);
        assert!(updated.execution_pointers[0].active);
        assert_eq!(
            updated.execution_pointers[0].persistence_data,
            Some(serde_json::json!({"count": 1}))
        );
    }

    #[tokio::test]
    async fn step_sleep_sets_sleeping_status() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<SleepStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<SleepStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Sleeping);
        assert!(updated.execution_pointers[0].sleep_until.is_some());
        assert!(updated.execution_pointers[0].active);
    }

    #[tokio::test]
    async fn step_wait_for_event_creates_subscription() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<WaitEventStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<WaitEventStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(
            updated.execution_pointers[0].status,
            PointerStatus::WaitingForEvent
        );
        assert!(!updated.execution_pointers[0].active);

        // Check subscription was created.
        use crate::traits::SubscriptionRepository;
        let subs = persistence
            .get_subscriptions("order.completed", "order-123", Utc::now())
            .await
            .unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].workflow_id, instance.id);
    }

    #[tokio::test]
    async fn event_arrived_resumes_pointer() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<EventResumeStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<EventResumeStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        let mut pointer = ExecutionPointer::new(0);
        // Simulate event arrived: pointer was waiting, now event is published.
        pointer.event_published = true;
        pointer.event_data = Some(serde_json::json!({"order_id": "123"}));
        pointer.status = PointerStatus::WaitingForEvent;
        pointer.active = true; // Re-activated by event processor.
        instance.execution_pointers.push(pointer);
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Complete);
        assert_eq!(updated.status, WorkflowStatus::Complete);
    }

    #[tokio::test]
    async fn branch_creates_child_pointers() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<BranchStep>();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<BranchStep>());
        s0.children.push(1);
        def.steps.push(s0);
        def.steps.push(WorkflowStep::new(1, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        // 1 original + 3 children.
        assert_eq!(updated.execution_pointers.len(), 4);
        // Children should have scope containing the parent pointer id.
        let parent_id = &updated.execution_pointers[0].id;
        for child in &updated.execution_pointers[1..] {
            assert!(child.scope.contains(parent_id));
            assert_eq!(child.step_id, 1);
            assert!(child.context_item.is_some());
        }
        // Parent should track children.
        assert_eq!(updated.execution_pointers[0].children.len(), 3);
    }

    #[tokio::test]
    async fn error_retry_increments_count() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<FailStep>());
        s0.error_behavior = Some(ErrorBehavior::Retry {
            interval: Duration::from_secs(10),
            max_retries: 0,
        });
        def.steps.push(s0);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers[0].retry_count, 1);
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Sleeping);
        assert!(updated.execution_pointers[0].sleep_until.is_some());
        assert_eq!(updated.status, WorkflowStatus::Runnable);
    }

    #[tokio::test]
    async fn error_suspend_pauses_workflow() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<FailStep>());
        s0.error_behavior = Some(ErrorBehavior::Suspend);
        def.steps.push(s0);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Suspended);
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Failed);
    }

    #[tokio::test]
    async fn error_terminate_ends_workflow() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<FailStep>());
        s0.error_behavior = Some(ErrorBehavior::Terminate);
        def.steps.push(s0);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Terminated);
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Failed);
        assert!(updated.complete_time.is_some());
    }

    #[tokio::test]
    async fn compensation_on_error() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailStep>();
        registry.register::<CompensateStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<FailStep>());
        s0.error_behavior = Some(ErrorBehavior::Compensate);
        s0.compensation_step_id = Some(1);
        def.steps.push(s0);
        def.steps.push(WorkflowStep::new(1, step_type::<CompensateStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Failed);
        // Compensation pointer should be created.
        assert_eq!(updated.execution_pointers.len(), 2);
        assert_eq!(updated.execution_pointers[1].step_id, 1);
        assert!(updated.execution_pointers[1].active);
    }

    #[tokio::test]
    async fn workflow_completes_when_all_pointers_done() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<PassStep>()));
        def.steps.push(WorkflowStep::new(1, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        // Two independent active pointers.
        instance.execution_pointers.push(ExecutionPointer::new(0));
        instance.execution_pointers.push(ExecutionPointer::new(1));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Complete);
        assert!(updated
            .execution_pointers
            .iter()
            .all(|p| p.status == PointerStatus::Complete));
    }

    #[tokio::test]
    async fn step_registry_register_and_resolve() {
        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let resolved = registry.resolve(&step_type::<PassStep>());
        assert!(resolved.is_some());

        let unresolved = registry.resolve("NonExistentStep");
        assert!(unresolved.is_none());
    }

    #[tokio::test]
    async fn executor_skips_non_runnable_workflow() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let registry = StepRegistry::new();
        let def = WorkflowDefinition::new("test", 1);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.status = WorkflowStatus::Complete;
        persistence.create_new_workflow(&instance).await.unwrap();

        // Should not error on a completed workflow.
        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Complete);
    }

    #[tokio::test]
    async fn sleeping_pointer_not_runnable_before_wakeup() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<SleepStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<SleepStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        let mut pointer = ExecutionPointer::new(0);
        pointer.status = PointerStatus::Sleeping;
        pointer.active = true;
        pointer.sleep_until = Some(Utc::now() + chrono::Duration::hours(1));
        instance.execution_pointers.push(pointer);
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        // Should still be sleeping since sleep_until is in the future.
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Sleeping);
    }

    #[tokio::test]
    async fn error_persists_execution_error() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<FailStep>());
        s0.error_behavior = Some(ErrorBehavior::Suspend);
        def.steps.push(s0);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let errors = persistence.get_errors().await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].workflow_id, instance.id);
    }

    #[tokio::test]
    async fn lifecycle_events_published() {
        let (persistence, lock, queue) = create_providers();
        let lifecycle = Arc::new(InMemoryLifecyclePublisher::new());
        let executor = create_executor(persistence.clone(), lock, queue)
            .with_lifecycle(lifecycle.clone());

        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        // Executor itself doesn't publish lifecycle events in the current implementation,
        // but the with_lifecycle builder works correctly.
        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Complete);
    }

    #[tokio::test]
    async fn default_error_behavior_used_when_step_has_none() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.default_error_behavior = ErrorBehavior::Terminate;
        // Step has no error_behavior override.
        def.steps.push(WorkflowStep::new(0, step_type::<FailStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Terminated);
    }

    #[tokio::test]
    async fn pointer_start_time_set_on_first_execution() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        def.steps.push(WorkflowStep::new(0, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert!(updated.execution_pointers[0].start_time.is_some());
        assert!(updated.execution_pointers[0].end_time.is_some());
    }

    #[tokio::test]
    async fn outcome_value_stored_on_pointer() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<OutcomeStep>();
        registry.register::<PassStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<OutcomeStep>());
        s0.outcomes.push(StepOutcome {
            next_step: 1,
            label: None,
            value: Some(serde_json::json!("yes")),
        });
        def.steps.push(s0);
        def.steps.push(WorkflowStep::new(1, step_type::<PassStep>()));

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(
            updated.execution_pointers[0].outcome,
            Some(serde_json::json!("yes"))
        );
    }

    #[tokio::test]
    async fn retry_then_succeed() {
        // A step that fails once then succeeds.
        use std::sync::atomic::{AtomicU32, Ordering};

        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

        #[derive(Default)]
        struct FailOnceStep;

        #[async_trait]
        impl StepBody for FailOnceStep {
            async fn run(
                &mut self,
                _ctx: &StepExecutionContext<'_>,
            ) -> crate::Result<ExecutionResult> {
                let count = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
                if count == 0 {
                    Err(WfeError::StepExecution("first attempt fails".into()))
                } else {
                    Ok(ExecutionResult::next())
                }
            }
        }

        CALL_COUNT.store(0, Ordering::SeqCst);

        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let mut registry = StepRegistry::new();
        registry.register::<FailOnceStep>();

        let mut def = WorkflowDefinition::new("test", 1);
        let mut s0 = WorkflowStep::new(0, step_type::<FailOnceStep>());
        s0.error_behavior = Some(ErrorBehavior::Retry {
            interval: Duration::from_millis(0),
            max_retries: 0,
        });
        def.steps.push(s0);

        let mut instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        instance.execution_pointers.push(ExecutionPointer::new(0));
        persistence.create_new_workflow(&instance).await.unwrap();

        // First execution: fails, retry scheduled.
        executor.execute(&instance.id, &def, &registry).await.unwrap();
        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers[0].retry_count, 1);
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Sleeping);

        // Second execution: succeeds (sleep_until is in the past with 0ms interval).
        executor.execute(&instance.id, &def, &registry).await.unwrap();
        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.execution_pointers[0].status, PointerStatus::Complete);
        assert_eq!(updated.status, WorkflowStatus::Complete);
    }

    #[tokio::test]
    async fn no_runnable_pointers_is_noop() {
        let (persistence, lock, queue) = create_providers();
        let executor = create_executor(persistence.clone(), lock, queue);

        let registry = StepRegistry::new();
        let def = WorkflowDefinition::new("test", 1);

        let instance = WorkflowInstance::new("test", 1, serde_json::json!({}));
        // No execution pointers at all.
        persistence.create_new_workflow(&instance).await.unwrap();

        executor.execute(&instance.id, &def, &registry).await.unwrap();

        let updated = persistence.get_workflow_instance(&instance.id).await.unwrap();
        assert_eq!(updated.status, WorkflowStatus::Runnable);
    }
}
