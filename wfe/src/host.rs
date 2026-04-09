use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use wfe_core::builder::WorkflowBuilder;
use wfe_core::executor::{StepRegistry, WorkflowExecutor};
use wfe_core::models::{
    Event, ExecutionPointer, LifecycleEvent, LifecycleEventType, PointerStatus, QueueType,
    WorkflowDefinition, WorkflowInstance, WorkflowStatus,
};
use wfe_core::traits::registry::WorkflowRegistry;
use wfe_core::traits::{
    DistributedLockProvider, HostContext, LifecyclePublisher, PersistenceProvider, QueueProvider,
    SearchIndex, ServiceProvider, StepBody, WorkflowData,
};
use wfe_core::{Result, WfeError};

use crate::registry::InMemoryWorkflowRegistry;

/// A lightweight HostContext implementation that delegates to the WorkflowHost's
/// components. Used by the background consumer task which cannot hold a direct
/// reference to WorkflowHost (it runs in a spawned tokio task).
pub(crate) struct HostContextImpl {
    persistence: Arc<dyn PersistenceProvider>,
    registry: Arc<RwLock<InMemoryWorkflowRegistry>>,
    queue_provider: Arc<dyn QueueProvider>,
}

impl HostContext for HostContextImpl {
    fn start_workflow(
        &self,
        definition_id: &str,
        version: u32,
        data: serde_json::Value,
        parent_root_workflow_id: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        let def_id = definition_id.to_string();
        let parent_root = parent_root_workflow_id;
        Box::pin(async move {
            // Look up the definition.
            let reg = self.registry.read().await;
            let definition = reg.get_definition(&def_id, Some(version)).ok_or_else(|| {
                WfeError::DefinitionNotFound {
                    id: def_id.clone(),
                    version,
                }
            })?;

            // Create the child workflow instance.
            let mut instance = WorkflowInstance::new(&def_id, version, data);
            if !definition.steps.is_empty() {
                instance.execution_pointers.push(ExecutionPointer::new(0));
            }

            // Inherit the parent's root so every descendant of a given
            // top-level workflow lands in the same Kubernetes namespace
            // and can share a provisioned volume.
            instance.root_workflow_id = parent_root;

            // Auto-assign a human-friendly name before persisting so the
            // child shows up as `{definition_id}-{N}` in lookups and logs.
            // Sub-workflows always use the default; callers wanting a custom
            // name should start the parent workflow directly.
            let n = self.persistence.next_definition_sequence(&def_id).await?;
            instance.name = format!("{def_id}-{n}");

            let id = self.persistence.create_new_workflow(&instance).await?;

            // Queue for execution.
            self.queue_provider
                .queue_work(&id, QueueType::Workflow)
                .await?;

            Ok(id)
        })
    }
}

/// The main orchestrator that ties all workflow engine components together.
pub struct WorkflowHost {
    pub(crate) persistence: Arc<dyn PersistenceProvider>,
    pub(crate) lock_provider: Arc<dyn DistributedLockProvider>,
    pub(crate) queue_provider: Arc<dyn QueueProvider>,
    pub(crate) lifecycle: Option<Arc<dyn LifecyclePublisher>>,
    pub(crate) search: Option<Arc<dyn SearchIndex>>,
    pub(crate) registry: Arc<RwLock<InMemoryWorkflowRegistry>>,
    pub(crate) step_registry: Arc<RwLock<StepRegistry>>,
    pub(crate) service_provider: Option<Arc<dyn ServiceProvider>>,
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
        sr.register::<sub_workflow::SubWorkflowStep>();
    }

    /// Spawn background polling tasks for processing workflows and events.
    pub async fn start(&self) -> Result<()> {
        self.register_primitives().await;
        self.persistence.ensure_store_exists().await?;
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
        let host_ctx = Arc::new(HostContextImpl {
            persistence: Arc::clone(&self.persistence),
            registry: Arc::clone(&self.registry),
            queue_provider: Arc::clone(&self.queue_provider),
        });
        let svc_provider = self.service_provider.clone();

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

                                        // Capability check: can this host execute this workflow?
                                        {
                                            let sr = step_registry.read().await;
                                            if !can_execute_workflow(&def_clone, &sr, &svc_provider) {
                                                drop(reg);
                                                debug!(
                                                    workflow_id = %workflow_id,
                                                    "Host cannot execute workflow, re-queuing"
                                                );
                                                if let Err(e) = queue.queue_work(&workflow_id, QueueType::Workflow).await {
                                                    error!(error = %e, "Failed to re-queue workflow");
                                                }
                                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                                continue;
                                            }
                                        }

                                        // Provision services if needed.
                                        if !def_clone.services.is_empty() {
                                            if let Some(ref provider) = svc_provider {
                                                match provider.provision(&workflow_id, &def_clone.services).await {
                                                    Ok(endpoints) => {
                                                        debug!(
                                                            workflow_id = %workflow_id,
                                                            services = endpoints.len(),
                                                            "Services provisioned"
                                                        );
                                                        // Inject service endpoints into workflow data.
                                                        if let Err(e) = inject_service_endpoints(
                                                            &executor.persistence,
                                                            &workflow_id,
                                                            &endpoints,
                                                        ).await {
                                                            error!(error = %e, "Failed to inject service endpoints");
                                                            provider.teardown(&workflow_id).await.ok();
                                                            continue;
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error!(
                                                            workflow_id = %workflow_id,
                                                            error = %e,
                                                            "Failed to provision services"
                                                        );
                                                        continue;
                                                    }
                                                }
                                            }
                                        }

                                        // Execute the workflow.
                                        let sr = step_registry.read().await;
                                        if let Err(e) = executor.execute(&workflow_id, &def_clone, &sr, Some(host_ctx.as_ref())).await {
                                            error!(workflow_id = %workflow_id, error = %e, "Workflow execution failed");
                                        }

                                        // Teardown services after execution (always, even on error).
                                        if !def_clone.services.is_empty() {
                                            if let Some(ref provider) = svc_provider {
                                                if let Err(e) = provider.teardown(&workflow_id).await {
                                                    warn!(workflow_id = %workflow_id, error = %e, "Service teardown failed");
                                                }
                                            }
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

    /// Register a step factory with an explicit key and factory function.
    /// Used by wfe-yaml and other dynamic step sources.
    pub async fn register_step_factory(
        &self,
        key: &str,
        factory: impl Fn() -> Box<dyn StepBody> + Send + Sync + 'static,
    ) {
        let mut sr = self.step_registry.write().await;
        sr.register_factory(key, factory);
    }

    /// Start a new workflow instance. The host auto-assigns a human-friendly
    /// name of the form `{definition_id}-{N}`. Use `start_workflow_with_name`
    /// to supply a caller-specified override.
    pub async fn start_workflow(
        &self,
        definition_id: &str,
        version: u32,
        data: serde_json::Value,
    ) -> Result<String> {
        self.start_workflow_with_name(definition_id, version, data, None)
            .await
    }

    /// Start a new workflow instance with an optional caller-supplied
    /// human-friendly name. When `name_override` is `None` the host
    /// auto-assigns `{definition_id}-{N}` using a per-definition sequence.
    #[tracing::instrument(
        name = "workflow.start",
        skip(self, data, name_override),
        fields(
            definition_id = %definition_id,
            version,
            workflow.definition_id = %definition_id,
            workflow.version = version,
            workflow.id = tracing::field::Empty,
            workflow.name = tracing::field::Empty,
        )
    )]
    pub async fn start_workflow_with_name(
        &self,
        definition_id: &str,
        version: u32,
        data: serde_json::Value,
        name_override: Option<String>,
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
            let mut pointer = ExecutionPointer::new(0);
            pointer.step_name = definition.steps.first().and_then(|s| s.name.clone());
            instance.execution_pointers.push(pointer);
        }

        // If the definition declares a shared volume, stash it in the
        // instance's data under a reserved key so sub-workflows inherit it
        // automatically (via the parent-data-inheritance in SubWorkflowStep).
        // The K8s executor reads it from workflow.data when
        // context.definition.shared_volume is None.
        if let Some(sv) = &definition.shared_volume {
            if let Some(obj) = instance.data.as_object_mut() {
                obj.insert(
                    "_wfe_shared_volume".to_string(),
                    serde_json::to_value(sv).unwrap_or_default(),
                );
            }
        }

        // Assign a human-friendly name. Callers may override (e.g. webhook
        // handlers that want `ci-mainline-a1b2c3`); otherwise use the
        // sequenced default. Validation: reject empty overrides so the name
        // column invariant holds.
        instance.name = match name_override {
            Some(n) if !n.trim().is_empty() => n,
            Some(_) => {
                return Err(WfeError::StepExecution(
                    "workflow name override must be non-empty".to_string(),
                ));
            }
            None => {
                let n = self
                    .persistence
                    .next_definition_sequence(definition_id)
                    .await?;
                format!("{definition_id}-{n}")
            }
        };

        // Persist the instance.
        let id = self.persistence.create_new_workflow(&instance).await?;
        instance.id = id.clone();
        tracing::Span::current().record("workflow.id", id.as_str());
        tracing::Span::current().record("workflow.name", instance.name.as_str());

        info!(
            workflow_id = %id,
            workflow_name = %instance.name,
            "Workflow instance created"
        );

        // Queue for execution.
        self.queue_provider
            .queue_work(&id, QueueType::Workflow)
            .await?;

        // Publish lifecycle event.
        if let Some(ref publisher) = self.lifecycle {
            let _ = publisher
                .publish(LifecycleEvent::new(
                    &id,
                    definition_id,
                    version,
                    LifecycleEventType::Started,
                ))
                .await;
        }

        Ok(id)
    }

    /// Publish an event that may resume waiting workflows.
    #[tracing::instrument(
        name = "event.publish",
        skip(self, data),
        fields(
            event.name = %event_name,
            event.key = %event_key,
        )
    )]
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
    pub async fn suspend_workflow(&self, id_or_name: &str) -> Result<bool> {
        let mut instance = self.get_workflow(id_or_name).await?;
        if instance.status != WorkflowStatus::Runnable {
            return Ok(false);
        }
        instance.status = WorkflowStatus::Suspended;
        self.persistence.persist_workflow(&instance).await?;
        if let Some(ref publisher) = self.lifecycle {
            let _ = publisher
                .publish(LifecycleEvent::new(
                    &instance.id,
                    &instance.workflow_definition_id,
                    instance.version,
                    LifecycleEventType::Suspended,
                ))
                .await;
        }
        Ok(true)
    }

    /// Resume a suspended workflow.
    pub async fn resume_workflow(&self, id_or_name: &str) -> Result<bool> {
        let mut instance = self.get_workflow(id_or_name).await?;
        if instance.status != WorkflowStatus::Suspended {
            return Ok(false);
        }
        instance.status = WorkflowStatus::Runnable;
        self.persistence.persist_workflow(&instance).await?;

        // Re-queue for execution using the canonical UUID (queue keys are
        // always UUIDs, never names).
        self.queue_provider
            .queue_work(&instance.id, QueueType::Workflow)
            .await?;

        if let Some(ref publisher) = self.lifecycle {
            let _ = publisher
                .publish(LifecycleEvent::new(
                    &instance.id,
                    &instance.workflow_definition_id,
                    instance.version,
                    LifecycleEventType::Resumed,
                ))
                .await;
        }
        Ok(true)
    }

    /// Terminate a running workflow.
    pub async fn terminate_workflow(&self, id_or_name: &str) -> Result<bool> {
        let mut instance = self.get_workflow(id_or_name).await?;
        if instance.status == WorkflowStatus::Complete
            || instance.status == WorkflowStatus::Terminated
        {
            return Ok(false);
        }
        instance.status = WorkflowStatus::Terminated;
        instance.complete_time = Some(chrono::Utc::now());
        self.persistence.persist_workflow(&instance).await?;
        if let Some(ref publisher) = self.lifecycle {
            let _ = publisher
                .publish(LifecycleEvent::new(
                    &instance.id,
                    &instance.workflow_definition_id,
                    instance.version,
                    LifecycleEventType::Terminated,
                ))
                .await;
        }
        Ok(true)
    }

    /// Fetch a workflow instance by UUID or human-friendly name.
    ///
    /// Tries UUID lookup first for the common case. On `WorkflowNotFound`,
    /// falls back to name lookup so callers can address instances
    /// interchangeably (e.g. `ci-42` or the UUID it was assigned).
    pub async fn get_workflow(&self, id_or_name: &str) -> Result<WorkflowInstance> {
        match self.persistence.get_workflow_instance(id_or_name).await {
            Ok(w) => Ok(w),
            Err(WfeError::WorkflowNotFound(_)) => {
                self.persistence
                    .get_workflow_instance_by_name(id_or_name)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    /// Resolve an identifier (UUID or human-friendly name) to the canonical
    /// UUID. Used by mutation APIs that still take `&str id` internally.
    pub async fn resolve_workflow_id(&self, id_or_name: &str) -> Result<String> {
        let instance = self.get_workflow(id_or_name).await?;
        Ok(instance.id)
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
#[tracing::instrument(
    name = "event.process",
    skip(persistence, lock_provider, queue),
    fields(event.id = %event_id)
)]
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

/// Check if a workflow definition can be executed by this host.
///
/// Returns false if:
/// - Any step type is not registered in the step registry
/// - Services are declared but no ServiceProvider is configured
/// - The ServiceProvider cannot provision the required services
fn can_execute_workflow(
    definition: &WorkflowDefinition,
    step_registry: &StepRegistry,
    service_provider: &Option<Arc<dyn ServiceProvider>>,
) -> bool {
    // Check all step types are registered.
    for step in &definition.steps {
        if step_registry.resolve(&step.step_type).is_none() {
            debug!(
                step_type = %step.step_type,
                "step type not registered on this host"
            );
            return false;
        }
    }

    // If services are declared, check service provider.
    if !definition.services.is_empty() {
        match service_provider {
            None => {
                debug!("workflow requires services but no ServiceProvider configured");
                return false;
            }
            Some(provider) => {
                if !provider.can_provision(&definition.services) {
                    debug!("ServiceProvider cannot provision required services");
                    return false;
                }
            }
        }
    }

    true
}

/// Inject service endpoint information into workflow instance data.
async fn inject_service_endpoints(
    persistence: &Arc<dyn PersistenceProvider>,
    workflow_id: &str,
    endpoints: &[wfe_core::models::ServiceEndpoint],
) -> Result<()> {
    let mut instance = persistence.get_workflow_instance(workflow_id).await?;

    // Build service info map.
    let mut services_map = serde_json::Map::new();
    for ep in endpoints {
        let mut ep_map = serde_json::Map::new();
        ep_map.insert("host".into(), serde_json::Value::String(ep.host.clone()));
        let ports: Vec<serde_json::Value> = ep
            .ports
            .iter()
            .map(|p| serde_json::json!(p.container_port))
            .collect();
        ep_map.insert("ports".into(), serde_json::Value::Array(ports));
        services_map.insert(ep.name.clone(), serde_json::Value::Object(ep_map));

        // Also set env-style keys: SVC_{NAME}_HOST, SVC_{NAME}_PORT
        let prefix = format!("SVC_{}", ep.name.to_uppercase());
        if let Some(data_obj) = instance.data.as_object_mut() {
            data_obj.insert(
                format!("{prefix}_HOST"),
                serde_json::Value::String(ep.host.clone()),
            );
            if let Some(port) = ep.ports.first() {
                data_obj.insert(
                    format!("{prefix}_PORT"),
                    serde_json::json!(port.container_port),
                );
            }
        }
    }

    if let Some(data_obj) = instance.data.as_object_mut() {
        data_obj.insert("services".into(), serde_json::Value::Object(services_map));
    }

    persistence.persist_workflow(&instance).await?;
    Ok(())
}
