use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::models::{Event, EventSubscription, ExecutionError, ScheduledCommand, WorkflowInstance};

/// Persistence for workflow instances.
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    /// Create a new workflow instance and return its generated id.
    async fn create_new_workflow(&self, instance: &WorkflowInstance) -> crate::Result<String>;
    /// Persist an updated workflow instance.
    async fn persist_workflow(&self, instance: &WorkflowInstance) -> crate::Result<()>;
    /// Persist a workflow instance together with new event subscriptions.
    async fn persist_workflow_with_subscriptions(
        &self,
        instance: &WorkflowInstance,
        subscriptions: &[EventSubscription],
    ) -> crate::Result<()>;
    /// Return ids of workflow instances that are runnable as of the given time.
    async fn get_runnable_instances(&self, as_at: DateTime<Utc>) -> crate::Result<Vec<String>>;
    /// Load a workflow instance by id.
    async fn get_workflow_instance(&self, id: &str) -> crate::Result<WorkflowInstance>;
    /// Load a workflow instance by its human-friendly name.
    async fn get_workflow_instance_by_name(&self, name: &str) -> crate::Result<WorkflowInstance>;
    /// Load multiple workflow instances by id.
    async fn get_workflow_instances(&self, ids: &[String]) -> crate::Result<Vec<WorkflowInstance>>;

    /// Atomically allocate the next sequence number for a given workflow
    /// definition id. Used by the host to assign human-friendly names of the
    /// form `{definition_id}-{N}` before inserting a new workflow instance.
    /// Guaranteed monotonic per definition_id; no guarantees across definitions.
    async fn next_definition_sequence(&self, definition_id: &str) -> crate::Result<u64>;
}

/// Persistence for event subscriptions.
#[async_trait]
pub trait SubscriptionRepository: Send + Sync {
    /// Create a new event subscription and return its generated id.
    async fn create_event_subscription(
        &self,
        subscription: &EventSubscription,
    ) -> crate::Result<String>;
    /// Return all active subscriptions matching the event name and key as of the given time.
    async fn get_subscriptions(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> crate::Result<Vec<EventSubscription>>;
    /// Mark a subscription as terminated so it no longer receives events.
    async fn terminate_subscription(&self, subscription_id: &str) -> crate::Result<()>;
    /// Load a subscription by id.
    async fn get_subscription(&self, subscription_id: &str) -> crate::Result<EventSubscription>;
    /// Return the first open subscription matching the event name and key, if any.
    async fn get_first_open_subscription(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> crate::Result<Option<EventSubscription>>;
    /// Set a processing token on a subscription for claim-based concurrency control.
    async fn set_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
        worker_id: &str,
        expiry: DateTime<Utc>,
    ) -> crate::Result<bool>;
    /// Clear a previously set processing token.
    async fn clear_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
    ) -> crate::Result<()>;
}

/// Persistence for events.
#[async_trait]
pub trait EventRepository: Send + Sync {
    /// Create a new event and return its generated id.
    async fn create_event(&self, event: &Event) -> crate::Result<String>;
    /// Load an event by id.
    async fn get_event(&self, id: &str) -> crate::Result<Event>;
    /// Return ids of events that are runnable as of the given time.
    async fn get_runnable_events(&self, as_at: DateTime<Utc>) -> crate::Result<Vec<String>>;
    /// Return ids of events matching the event name and key as of the given time.
    async fn get_events(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> crate::Result<Vec<String>>;
    /// Mark an event as processed.
    async fn mark_event_processed(&self, id: &str) -> crate::Result<()>;
    /// Mark an event as unprocessed so it can be retried.
    async fn mark_event_unprocessed(&self, id: &str) -> crate::Result<()>;
}

/// Persistence for scheduled commands.
#[async_trait]
pub trait ScheduledCommandRepository: Send + Sync {
    /// Whether this backend supports scheduled commands.
    fn supports_scheduled_commands(&self) -> bool;
    /// Schedule a command to be processed later.
    async fn schedule_command(&self, command: &ScheduledCommand) -> crate::Result<()>;
    /// Process commands that are due as of the given time.
    async fn process_commands(
        &self,
        as_of: DateTime<Utc>,
        handler: &(
             dyn Fn(
            ScheduledCommand,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::Result<()>> + Send>,
        > + Send
                 + Sync
         ),
    ) -> crate::Result<()>;
}

/// Composite persistence provider combining all repository traits.
#[async_trait]
pub trait PersistenceProvider:
    WorkflowRepository + EventRepository + SubscriptionRepository + ScheduledCommandRepository
{
    /// Persist execution errors associated with workflow pointers.
    async fn persist_errors(&self, errors: &[ExecutionError]) -> crate::Result<()>;
    /// Ensure the underlying store (tables, indexes, etc.) exists.
    async fn ensure_store_exists(&self) -> crate::Result<()>;
}
