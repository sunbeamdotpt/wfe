use async_trait::async_trait;

use crate::models::LifecycleEvent;

/// Publishes lifecycle events for workflow state transitions.
#[async_trait]
pub trait LifecyclePublisher: Send + Sync {
    async fn publish(&self, event: LifecycleEvent) -> crate::Result<()>;
}
