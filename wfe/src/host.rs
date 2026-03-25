use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use wfe_core::executor::{StepRegistry, WorkflowExecutor};
use wfe_core::models::{
    Event, ExecutionPointer, PointerStatus, QueueType, WorkflowDefinition, WorkflowInstance,
    WorkflowStatus,
};
use wfe_core::traits::{
    DistributedLockProvider, LifecyclePublisher, PersistenceProvider, QueueProvider, SearchIndex,
    StepBody, WorkflowData,
};
use wfe_core::traits::registry::WorkflowRegistry;
use wfe_core::{Result, WfeError};
use wfe_core::builder::WorkflowBuilder;

use crate::registry::InMemoryWorkflowRegistry;

/// The main orchestrator that ties all workflow engine components together.
pub struct WorkflowHost {
    pub(crate) persistence: Arc<dyn PersistenceProvider>,
    pub(crate) lock_provider: Arc<dyn DistributedLockProvider>,
    pub(crate) queue_provider: Arc<dyn QueueProvider>,
    pub(crate) lifecycle: Option<Arc<dyn LifecyclePublisher>>,
    pub(crate) search: Option<Arc<dyn SearchIndex>>,
    pub(crate) registry: Arc<RwLock<InMemoryWorkflowRegistry>>,
    pub(crate) step_registry: Arc<RwLock<StepRegistry>>,
    pub(crate) executor: Arc<WorkflowExecutor>,
    pub(crate) shutdown: CancellationToken,
}

impl WorkflowHost {
    /// Register all built-in primitive step types.
    async fn register_primitives(&self) {
        use wfe_core::primitives::*;
        let mut sr = self.step_registry.write().await;
        sr.register::<decide::DecideStep>();
        sr.register::<delay::DelayStep>();
        sr.register::<end_step::EndStep>();
        sr.register::<foreach_step::ForEachStep>();
        sr.register::<if_step::IfStep>();
        sr.register::<poll_endpoint::PollEndpointStep>();
        sr.register::<recur::RecurStep>();
        sr.register::<saga_container::SagaContainerStep>();
        sr.register::<schedule::ScheduleStep>();
        sr.register::<sequence::SequenceStep>();
        sr.register::<wait_for::WaitForStep>();
        sr.register::<while_step::WhileStep>();
    }

    /// Spawn background polling tasks for processing workflows and events.
    pub async fn start(&self) -> Result<()> {
        self.register_primitives().await;
        self.queue_provider.start().await?;
        self.lock_provider.start().await?;
        if let Some(ref search) = self.search {
            search.start().await?;
        }

        // Spawn workflow consumer task.
        let executor = Arc::clone(&self.executor);
        let registry = Arc::clone(&self.registry);
        let step_registry = Arc::clone(&self.step_registry);
        let queue = Arc::clone(&self.queue_provider);
        let shutdown = self.shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        debug!("Workflow consumer shutting down");
                        break;
                    }
                    result = queue.dequeue_work(QueueType::Workflow) => {
                        match result {
                            Ok(Some(workflow_id)) => {
                                // Look up the workflow instance to find its definition.
                                let instance = match executor.persistence.get_workflow_instance(&workflow_id).await {
                                    Ok(inst) => inst,
                                    Err(e) => {
                                        error!(workflow_id = %workflow_id, error = %e, "Failed to load workflow instance");
                                        continue;
                                    }
                                };
                                let reg = registry.read().await;
                                let definition = reg.get_definition(
                                    &instance.workflow_definition_id,
                                    Some(instance.version),
                                );
                                match definition {
                                    Some(def) => {
                                        let def_clone = def.clone();
                                        let sr = step_registry.read().await;
                                        if let Err(e) = executor.execute(&workflow_id, &def_clone, &sr).await {
                                            error!(workflow_id = %workflow_id, error = %e, "Workflow execution failed");
                                        }
                                    }
                                    None => {
                                        warn!(
                                            workflow_id = %workflow_id,
                                            definition_id = %instance.workflow_definition_id,
                                            version = instance.version,
                                            "Workflow definition not found"
                                        );
                                    }
                                }
                            }
                            Ok(None) => {
                                // No work available; sleep briefly before polling again.
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to dequeue workflow work");
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
        });

        // Spawn event consumer task.
        let persistence = Arc::clone(&self.persistence);
        let lock_provider2 = Arc::clone(&self.lock_provider);
        let queue2 = Arc::clone(&self.queue_provider);
        let shutdown2 = self.shutdown.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown2.cancelled() => {
                        debug!("Event consumer shutting down");
                        break;
                    }
                    result = queue2.dequeue_work(QueueType::Event) => {
                        match result {
                            Ok(Some(event_id)) => {
                                if let Err(e) = process_event(&persistence, &lock_provider2, &queue2, &event_id).await {
                                    error!(event_id = %event_id, error = %e, "Event processing failed");
                                }
                            }
                            Ok(None) => {
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                            }
                            Err(e) => {
                                error!(error = %e, "Failed to dequeue event work");
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    /// Signal shutdown of all background tasks.
    pub async fn stop(&self) {
        self.shutdown.cancel();
        if let Err(e) = self.queue_provider.stop().await {
            warn!(error = %e, "Failed to stop queue provider");
        }
        if let Err(e) = self.lock_provider.stop().await {
            warn!(error = %e, "Failed to stop lock provider");
        }
        if let Some(ref search) = self.search
            && let Err(e) = search.stop().await
        {
            warn!(error = %e, "Failed to stop search index");
        }
    }

    /// Register a workflow definition built via a closure that configures a `WorkflowBuilder`.
    pub async fn register_workflow<D: WorkflowData>(
        &self,
        builder_fn: &dyn Fn(WorkflowBuilder<D>) -> WorkflowBuilder<D>,
        id: &str,
        version: u32,
    ) -> WorkflowDefinition {
        let builder = WorkflowBuilder::<D>::new();
        let builder = builder_fn(builder);
        let definition = builder.build(id, version);
        let mut reg = self.registry.write().await;
        reg.register(definition.clone());
        definition
    }

    /// Register a pre-built `WorkflowDefinition` directly.
    pub async fn register_workflow_definition(&self, definition: WorkflowDefinition) {
        let mut reg = self.registry.write().await;
        reg.register(definition);
    }

    /// Register a step type with the step registry.
    pub async fn register_step<S: StepBody + Default + 'static>(&self) {
        let mut sr = self.step_registry.write().await;
        sr.register::<S>();
    }

    /// Start a new workflow instance.
    pub async fn start_workflow(
        &self,
        definition_id: &str,
        version: u32,
        data: serde_json::Value,
    ) -> Result<String> {
        // Verify definition exists.
        let reg = self.registry.read().await;
        let definition = reg
            .get_definition(definition_id, Some(version))
            .ok_or_else(|| WfeError::DefinitionNotFound {
                id: definition_id.to_string(),
                version,
            })?;

        // Create initial execution pointer for step 0 if the definition has steps.
        let mut instance = WorkflowInstance::new(definition_id, version, data);
        if !definition.steps.is_empty() {
            instance.execution_pointers.push(ExecutionPointer::new(0));
        }

        // Persist the instance.
        let id = self.persistence.create_new_workflow(&instance).await?;
        instance.id = id.clone();

        // Queue for execution.
        self.queue_provider
            .queue_work(&id, QueueType::Workflow)
            .await?;

        Ok(id)
    }

    /// Publish an event that may resume waiting workflows.
    pub async fn publish_event(
        &self,
        event_name: &str,
        event_key: &str,
        data: serde_json::Value,
    ) -> Result<()> {
        let event = Event::new(event_name, event_key, data);
        let event_id = self.persistence.create_event(&event).await?;

        // Queue event for processing.
        self.queue_provider
            .queue_work(&event_id, QueueType::Event)
            .await?;

        Ok(())
    }

    /// Suspend a running workflow.
    pub async fn suspend_workflow(&self, id: &str) -> Result<bool> {
        let mut instance = self.persistence.get_workflow_instance(id).await?;
        if instance.status != WorkflowStatus::Runnable {
            return Ok(false);
        }
        instance.status = WorkflowStatus::Suspended;
        self.persistence.persist_workflow(&instance).await?;
        Ok(true)
    }

    /// Resume a suspended workflow.
    pub async fn resume_workflow(&self, id: &str) -> Result<bool> {
        let mut instance = self.persistence.get_workflow_instance(id).await?;
        if instance.status != WorkflowStatus::Suspended {
            return Ok(false);
        }
        instance.status = WorkflowStatus::Runnable;
        self.persistence.persist_workflow(&instance).await?;

        // Re-queue for execution.
        self.queue_provider
            .queue_work(id, QueueType::Workflow)
            .await?;

        Ok(true)
    }

    /// Terminate a running workflow.
    pub async fn terminate_workflow(&self, id: &str) -> Result<bool> {
        let mut instance = self.persistence.get_workflow_instance(id).await?;
        if instance.status == WorkflowStatus::Complete
            || instance.status == WorkflowStatus::Terminated
        {
            return Ok(false);
        }
        instance.status = WorkflowStatus::Terminated;
        instance.complete_time = Some(chrono::Utc::now());
        self.persistence.persist_workflow(&instance).await?;
        Ok(true)
    }

    /// Fetch a workflow instance by ID.
    pub async fn get_workflow(&self, id: &str) -> Result<WorkflowInstance> {
        self.persistence.get_workflow_instance(id).await
    }

    /// Access the persistence provider.
    pub fn persistence(&self) -> &Arc<dyn PersistenceProvider> {
        &self.persistence
    }

    /// Access the lifecycle publisher, if configured.
    pub fn lifecycle(&self) -> Option<&Arc<dyn LifecyclePublisher>> {
        self.lifecycle.as_ref()
    }
}

/// Process an event: find matching subscriptions, set event_data on pointers, re-queue workflows.
async fn process_event(
    persistence: &Arc<dyn PersistenceProvider>,
    lock_provider: &Arc<dyn DistributedLockProvider>,
    queue: &Arc<dyn QueueProvider>,
    event_id: &str,
) -> Result<()> {
    let event = persistence.get_event(event_id).await?;

    // Find matching subscriptions.
    let subscriptions = persistence
        .get_subscriptions(&event.event_name, &event.event_key, event.event_time)
        .await?;

    for sub in &subscriptions {
        // Acquire lock on the workflow to prevent concurrent modifications.
        if !lock_provider.acquire_lock(&sub.workflow_id).await? {
            // Re-queue the event for retry
            queue.queue_work(event_id, QueueType::Event).await?;
            return Ok(());
        }

        let result = async {
            // Load the workflow and update the matching execution pointer.
            let mut instance = persistence.get_workflow_instance(&sub.workflow_id).await?;

            for pointer in &mut instance.execution_pointers {
                if pointer.id == sub.execution_pointer_id
                    && pointer.status == PointerStatus::WaitingForEvent
                {
                    pointer.event_data = Some(event.event_data.clone());
                    pointer.event_published = true;
                    pointer.active = true;
                }
            }

            instance.next_execution = Some(0);
            persistence.persist_workflow(&instance).await?;

            // Re-queue the workflow for execution.
            queue
                .queue_work(&sub.workflow_id, QueueType::Workflow)
                .await?;

            // Terminate the subscription.
            persistence.terminate_subscription(&sub.id).await?;

            Ok::<(), WfeError>(())
        }
        .await;

        // Release lock regardless of outcome.
        lock_provider.release_lock(&sub.workflow_id).await?;

        result?;
    }

    // Mark the event as processed.
    persistence.mark_event_processed(event_id).await?;

    Ok(())
}
