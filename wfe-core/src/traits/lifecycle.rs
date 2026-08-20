use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::models::LifecycleEvent;

/// Publishes lifecycle events for workflow state transitions.
#[async_trait]
pub trait LifecyclePublisher: Send + Sync {
    /// Publish a lifecycle event.
    async fn publish(&self, event: LifecycleEvent) -> crate::Result<()>;

    /// Subscribe to lifecycle events.
    ///
    /// Backends that do not support subscriptions return an error by default.
    fn subscribe(&self) -> crate::Result<broadcast::Receiver<LifecycleEvent>> {
        Err(crate::WfeError::Other(
            "subscriptions are not supported by this lifecycle publisher".into(),
        ))
    }
}
