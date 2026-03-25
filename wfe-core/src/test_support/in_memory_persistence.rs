use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::RwLock;

use crate::models::{
    Event, EventSubscription, ExecutionError, ScheduledCommand, WorkflowInstance,
};
use crate::traits::{
    EventRepository, PersistenceProvider, ScheduledCommandRepository, SubscriptionRepository,
    WorkflowRepository,
};
use crate::{Result, WfeError};

/// An in-memory implementation of `PersistenceProvider` for testing.
#[derive(Debug, Clone)]
pub struct InMemoryPersistenceProvider {
    workflows: Arc<RwLock<HashMap<String, WorkflowInstance>>>,
    events: Arc<RwLock<HashMap<String, Event>>>,
    subscriptions: Arc<RwLock<HashMap<String, EventSubscription>>>,
    errors: Arc<RwLock<Vec<ExecutionError>>>,
    scheduled_commands: Arc<RwLock<Vec<ScheduledCommand>>>,
}

impl InMemoryPersistenceProvider {
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            errors: Arc::new(RwLock::new(Vec::new())),
            scheduled_commands: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Retrieve all stored errors (for test assertions).
    pub async fn get_errors(&self) -> Vec<ExecutionError> {
        self.errors.read().await.clone()
    }
}

impl Default for InMemoryPersistenceProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl WorkflowRepository for InMemoryPersistenceProvider {
    async fn create_new_workflow(&self, instance: &WorkflowInstance) -> Result<String> {
        let id = if instance.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            instance.id.clone()
        };
        let mut stored = instance.clone();
        stored.id = id.clone();
        self.workflows.write().await.insert(id.clone(), stored);
        Ok(id)
    }

    async fn persist_workflow(&self, instance: &WorkflowInstance) -> Result<()> {
        self.workflows
            .write()
            .await
            .insert(instance.id.clone(), instance.clone());
        Ok(())
    }

    async fn persist_workflow_with_subscriptions(
        &self,
        instance: &WorkflowInstance,
        subscriptions: &[EventSubscription],
    ) -> Result<()> {
        self.persist_workflow(instance).await?;
        let mut subs = self.subscriptions.write().await;
        for sub in subscriptions {
            subs.insert(sub.id.clone(), sub.clone());
        }
        Ok(())
    }

    async fn get_runnable_instances(&self, as_at: DateTime<Utc>) -> Result<Vec<String>> {
        let workflows = self.workflows.read().await;
        let as_at_millis = as_at.timestamp_millis();
        let ids = workflows
            .values()
            .filter(|w| {
                w.status == crate::models::WorkflowStatus::Runnable
                    && w.next_execution
                        .map(|ne| ne <= as_at_millis)
                        .unwrap_or(false)
            })
            .map(|w| w.id.clone())
            .collect();
        Ok(ids)
    }

    async fn get_workflow_instance(&self, id: &str) -> Result<WorkflowInstance> {
        self.workflows
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| WfeError::WorkflowNotFound(id.to_string()))
    }

    async fn get_workflow_instances(&self, ids: &[String]) -> Result<Vec<WorkflowInstance>> {
        let workflows = self.workflows.read().await;
        let mut result = Vec::new();
        for id in ids {
            if let Some(w) = workflows.get(id) {
                result.push(w.clone());
            }
        }
        Ok(result)
    }
}

#[async_trait]
impl SubscriptionRepository for InMemoryPersistenceProvider {
    async fn create_event_subscription(
        &self,
        subscription: &EventSubscription,
    ) -> Result<String> {
        let id = if subscription.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            subscription.id.clone()
        };
        let mut stored = subscription.clone();
        stored.id = id.clone();
        self.subscriptions.write().await.insert(id.clone(), stored);
        Ok(id)
    }

    async fn get_subscriptions(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<EventSubscription>> {
        let subs = self.subscriptions.read().await;
        let result = subs
            .values()
            .filter(|s| {
                s.event_name == event_name
                    && s.event_key == event_key
                    && s.subscribe_as_of <= as_of
                    && s.external_token.is_none() // not terminated
            })
            .cloned()
            .collect();
        Ok(result)
    }

    async fn terminate_subscription(&self, subscription_id: &str) -> Result<()> {
        let mut subs = self.subscriptions.write().await;
        match subs.get_mut(subscription_id) {
            Some(sub) => {
                sub.external_token = Some("__terminated__".to_string());
                Ok(())
            }
            None => Err(WfeError::SubscriptionNotFound(subscription_id.to_string())),
        }
    }

    async fn get_subscription(&self, subscription_id: &str) -> Result<EventSubscription> {
        self.subscriptions
            .read()
            .await
            .get(subscription_id)
            .cloned()
            .ok_or_else(|| WfeError::SubscriptionNotFound(subscription_id.to_string()))
    }

    async fn get_first_open_subscription(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Option<EventSubscription>> {
        let subs = self.subscriptions.read().await;
        let result = subs
            .values()
            .find(|s| {
                s.event_name == event_name
                    && s.event_key == event_key
                    && s.subscribe_as_of <= as_of
                    && s.external_token.is_none()
            })
            .cloned();
        Ok(result)
    }

    async fn set_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
        worker_id: &str,
        expiry: DateTime<Utc>,
    ) -> Result<bool> {
        let mut subs = self.subscriptions.write().await;
        match subs.get_mut(subscription_id) {
            Some(sub) => {
                if sub.external_token.is_some() {
                    return Ok(false);
                }
                sub.external_token = Some(token.to_string());
                sub.external_worker_id = Some(worker_id.to_string());
                sub.external_token_expiry = Some(expiry);
                Ok(true)
            }
            None => Err(WfeError::SubscriptionNotFound(subscription_id.to_string())),
        }
    }

    async fn clear_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
    ) -> Result<()> {
        let mut subs = self.subscriptions.write().await;
        match subs.get_mut(subscription_id) {
            Some(sub) => {
                if sub.external_token.as_deref() != Some(token) {
                    return Err(WfeError::Persistence(format!(
                        "Token mismatch for subscription {subscription_id}"
                    )));
                }
                sub.external_token = None;
                sub.external_worker_id = None;
                sub.external_token_expiry = None;
                Ok(())
            }
            None => Err(WfeError::SubscriptionNotFound(subscription_id.to_string())),
        }
    }
}

#[async_trait]
impl EventRepository for InMemoryPersistenceProvider {
    async fn create_event(&self, event: &Event) -> Result<String> {
        let id = if event.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.id.clone()
        };
        let mut stored = event.clone();
        stored.id = id.clone();
        self.events.write().await.insert(id.clone(), stored);
        Ok(id)
    }

    async fn get_event(&self, id: &str) -> Result<Event> {
        self.events
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| WfeError::EventNotFound(id.to_string()))
    }

    async fn get_runnable_events(&self, as_at: DateTime<Utc>) -> Result<Vec<String>> {
        let events = self.events.read().await;
        let ids = events
            .values()
            .filter(|e| !e.is_processed && e.event_time <= as_at)
            .map(|e| e.id.clone())
            .collect();
        Ok(ids)
    }

    async fn get_events(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        let events = self.events.read().await;
        let ids = events
            .values()
            .filter(|e| e.event_name == event_name && e.event_key == event_key && e.event_time <= as_of)
            .map(|e| e.id.clone())
            .collect();
        Ok(ids)
    }

    async fn mark_event_processed(&self, id: &str) -> Result<()> {
        let mut events = self.events.write().await;
        match events.get_mut(id) {
            Some(event) => {
                event.is_processed = true;
                Ok(())
            }
            None => Err(WfeError::EventNotFound(id.to_string())),
        }
    }

    async fn mark_event_unprocessed(&self, id: &str) -> Result<()> {
        let mut events = self.events.write().await;
        match events.get_mut(id) {
            Some(event) => {
                event.is_processed = false;
                Ok(())
            }
            None => Err(WfeError::EventNotFound(id.to_string())),
        }
    }
}

#[async_trait]
impl ScheduledCommandRepository for InMemoryPersistenceProvider {
    fn supports_scheduled_commands(&self) -> bool {
        true
    }

    async fn schedule_command(&self, command: &ScheduledCommand) -> Result<()> {
        self.scheduled_commands.write().await.push(command.clone());
        Ok(())
    }

    async fn process_commands(
        &self,
        as_of: DateTime<Utc>,
        handler: &(dyn Fn(ScheduledCommand) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
              + Send
              + Sync),
    ) -> Result<()> {
        let as_of_millis = as_of.timestamp_millis();
        let due: Vec<ScheduledCommand> = {
            let mut cmds = self.scheduled_commands.write().await;
            let (due, remaining): (Vec<_>, Vec<_>) =
                cmds.drain(..).partition(|c| c.execute_time <= as_of_millis);
            *cmds = remaining;
            due
        };
        for cmd in due {
            handler(cmd).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl PersistenceProvider for InMemoryPersistenceProvider {
    async fn persist_errors(&self, errors: &[ExecutionError]) -> Result<()> {
        let mut stored = self.errors.write().await;
        stored.extend(errors.iter().cloned());
        Ok(())
    }

    async fn ensure_store_exists(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Event, EventSubscription, ExecutionError, ScheduledCommand, CommandName};
    use crate::traits::{
        EventRepository, PersistenceProvider, ScheduledCommandRepository, SubscriptionRepository,
        WorkflowRepository,
    };
    use chrono::{Duration, Utc};

    #[tokio::test]
    async fn default_impl() {
        let p = InMemoryPersistenceProvider::default();
        p.ensure_store_exists().await.unwrap();
    }

    #[tokio::test]
    async fn get_workflow_instances_batch() {
        let p = InMemoryPersistenceProvider::new();
        let w1 = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        let id1 = p.create_new_workflow(&w1).await.unwrap();
        let w2 = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        let id2 = p.create_new_workflow(&w2).await.unwrap();

        let ids = vec![id1.clone(), id2.clone(), "nonexistent".to_string()];
        let result = p.get_workflow_instances(&ids).await.unwrap();
        // Only existing workflows are returned.
        assert_eq!(result.len(), 2);
    }

    #[tokio::test]
    async fn get_first_open_subscription_returns_match() {
        let p = InMemoryPersistenceProvider::new();
        let now = Utc::now();
        let sub = EventSubscription::new("wf-1", 0, "ptr-1", "evt", "key", now);
        let id = p.create_event_subscription(&sub).await.unwrap();

        let found = p
            .get_first_open_subscription("evt", "key", now + Duration::seconds(1))
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, id);
    }

    #[tokio::test]
    async fn get_first_open_subscription_returns_none() {
        let p = InMemoryPersistenceProvider::new();
        let now = Utc::now();
        let found = p
            .get_first_open_subscription("evt", "key", now)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn set_subscription_token_success() {
        let p = InMemoryPersistenceProvider::new();
        let now = Utc::now();
        let sub = EventSubscription::new("wf-1", 0, "ptr-1", "evt", "key", now);
        let id = p.create_event_subscription(&sub).await.unwrap();

        let expiry = now + Duration::hours(1);
        let result = p
            .set_subscription_token(&id, "token-1", "worker-1", expiry)
            .await
            .unwrap();
        assert!(result);

        // Setting token again on already-tokened subscription returns false.
        let result2 = p
            .set_subscription_token(&id, "token-2", "worker-2", expiry)
            .await
            .unwrap();
        assert!(!result2);
    }

    #[tokio::test]
    async fn set_subscription_token_not_found() {
        let p = InMemoryPersistenceProvider::new();
        let result = p
            .set_subscription_token("nonexistent", "t", "w", Utc::now())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn clear_subscription_token_success() {
        let p = InMemoryPersistenceProvider::new();
        let now = Utc::now();
        let sub = EventSubscription::new("wf-1", 0, "ptr-1", "evt", "key", now);
        let id = p.create_event_subscription(&sub).await.unwrap();

        let expiry = now + Duration::hours(1);
        p.set_subscription_token(&id, "token-1", "worker-1", expiry)
            .await
            .unwrap();

        p.clear_subscription_token(&id, "token-1").await.unwrap();

        // After clearing, the subscription should be open again.
        let retrieved = p.get_subscription(&id).await.unwrap();
        assert!(retrieved.external_token.is_none());
    }

    #[tokio::test]
    async fn clear_subscription_token_not_found() {
        let p = InMemoryPersistenceProvider::new();
        let result = p.clear_subscription_token("nonexistent", "t").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn terminate_subscription_not_found() {
        let p = InMemoryPersistenceProvider::new();
        let result = p.terminate_subscription("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_events_by_name_and_key() {
        let p = InMemoryPersistenceProvider::new();
        let now = Utc::now();
        let e1 = Event::new("evt-a", "key-1", serde_json::json!({}));
        let id1 = p.create_event(&e1).await.unwrap();
        let e2 = Event::new("evt-a", "key-2", serde_json::json!({}));
        let _id2 = p.create_event(&e2).await.unwrap();
        let e3 = Event::new("evt-b", "key-1", serde_json::json!({}));
        let _id3 = p.create_event(&e3).await.unwrap();

        let ids = p
            .get_events("evt-a", "key-1", now + Duration::seconds(1))
            .await
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], id1);
    }

    #[tokio::test]
    async fn mark_event_unprocessed() {
        let p = InMemoryPersistenceProvider::new();
        let e = Event::new("evt", "key", serde_json::json!({}));
        let id = p.create_event(&e).await.unwrap();

        p.mark_event_processed(&id).await.unwrap();
        let event = p.get_event(&id).await.unwrap();
        assert!(event.is_processed);

        p.mark_event_unprocessed(&id).await.unwrap();
        let event = p.get_event(&id).await.unwrap();
        assert!(!event.is_processed);
    }

    #[tokio::test]
    async fn mark_event_unprocessed_not_found() {
        let p = InMemoryPersistenceProvider::new();
        let result = p.mark_event_unprocessed("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mark_event_processed_not_found() {
        let p = InMemoryPersistenceProvider::new();
        let result = p.mark_event_processed("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_errors_returns_persisted_errors() {
        let p = InMemoryPersistenceProvider::new();
        let errors = vec![
            ExecutionError::new("wf-1", "ptr-1", "err1"),
            ExecutionError::new("wf-2", "ptr-2", "err2"),
        ];
        p.persist_errors(&errors).await.unwrap();
        let stored = p.get_errors().await;
        assert_eq!(stored.len(), 2);
    }

    #[tokio::test]
    async fn supports_scheduled_commands_returns_true() {
        let p = InMemoryPersistenceProvider::new();
        assert!(p.supports_scheduled_commands());
    }

    #[tokio::test]
    async fn schedule_and_process_commands() {
        let p = InMemoryPersistenceProvider::new();
        let now = Utc::now();
        let cmd = ScheduledCommand {
            command_name: CommandName::ProcessEvent,
            data: "test-event-id".to_string(),
            execute_time: now.timestamp_millis() - 1000,
        };
        p.schedule_command(&cmd).await.unwrap();

        p.process_commands(now, &|c: ScheduledCommand| {
            let _data = c.data;
            Box::pin(async move { Ok(()) })
        })
        .await
        .unwrap();

        // After processing, no more due commands remain.
        let remaining_count = p.scheduled_commands.read().await.len();
        assert_eq!(remaining_count, 0);
    }

    #[tokio::test]
    async fn persist_workflow_with_subscriptions() {
        let p = InMemoryPersistenceProvider::new();
        let w = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        let id = p.create_new_workflow(&w).await.unwrap();
        let mut updated = p.get_workflow_instance(&id).await.unwrap();
        updated.description = Some("updated".to_string());

        let sub = EventSubscription::new("wf-1", 0, "ptr-1", "evt", "key", Utc::now());
        p.persist_workflow_with_subscriptions(&updated, &[sub])
            .await
            .unwrap();

        let retrieved = p.get_workflow_instance(&id).await.unwrap();
        assert_eq!(retrieved.description.as_deref(), Some("updated"));
    }
}
