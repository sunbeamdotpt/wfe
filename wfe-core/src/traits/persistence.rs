use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::models::{Event, EventSubscription, ExecutionError, ScheduledCommand, WorkflowInstance};

/// Persistence for workflow instances.
#[async_trait]
pub trait WorkflowRepository: Send + Sync {
    async fn create_new_workflow(&self, instance: &WorkflowInstance) -> crate::Result<String>;
    async fn persist_workflow(&self, instance: &WorkflowInstance) -> crate::Result<()>;
    async fn persist_workflow_with_subscriptions(
        &self,
        instance: &WorkflowInstance,
        subscriptions: &[EventSubscription],
    ) -> crate::Result<()>;
    async fn get_runnable_instances(&self, as_at: DateTime<Utc>) -> crate::Result<Vec<String>>;
    async fn get_workflow_instance(&self, id: &str) -> crate::Result<WorkflowInstance>;
    async fn get_workflow_instance_by_name(&self, name: &str) -> crate::Result<WorkflowInstance>;
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
    async fn create_event_subscription(
        &self,
        subscription: &EventSubscription,
    ) -> crate::Result<String>;
    async fn get_subscriptions(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> crate::Result<Vec<EventSubscription>>;
    async fn terminate_subscription(&self, subscription_id: &str) -> crate::Result<()>;
    async fn get_subscription(&self, subscription_id: &str) -> crate::Result<EventSubscription>;
    async fn get_first_open_subscription(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> crate::Result<Option<EventSubscription>>;
    async fn set_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
        worker_id: &str,
        expiry: DateTime<Utc>,
    ) -> crate::Result<bool>;
    async fn clear_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
    ) -> crate::Result<()>;
}

/// Persistence for events.
#[async_trait]
pub trait EventRepository: Send + Sync {
    async fn create_event(&self, event: &Event) -> crate::Result<String>;
    async fn get_event(&self, id: &str) -> crate::Result<Event>;
    async fn get_runnable_events(&self, as_at: DateTime<Utc>) -> crate::Result<Vec<String>>;
    async fn get_events(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> crate::Result<Vec<String>>;
    async fn mark_event_processed(&self, id: &str) -> crate::Result<()>;
    async fn mark_event_unprocessed(&self, id: &str) -> crate::Result<()>;
}

/// Persistence for scheduled commands.
#[async_trait]
pub trait ScheduledCommandRepository: Send + Sync {
    fn supports_scheduled_commands(&self) -> bool;
    async fn schedule_command(&self, command: &ScheduledCommand) -> crate::Result<()>;
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
    async fn persist_errors(&self, errors: &[ExecutionError]) -> crate::Result<()>;
    async fn ensure_store_exists(&self) -> crate::Result<()>;
}
