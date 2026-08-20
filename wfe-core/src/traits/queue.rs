use async_trait::async_trait;

use crate::models::QueueType;

/// Queue provider for distributing workflow execution across workers.
#[async_trait]
pub trait QueueProvider: Send + Sync {
    /// Enqueue an item (typically a workflow or event id) for later processing.
    async fn queue_work(&self, id: &str, queue: QueueType) -> crate::Result<()>;
    /// Dequeue the next item from the given queue. Returns `None` when no work is available.
    async fn dequeue_work(&self, queue: QueueType) -> crate::Result<Option<String>>;
    /// Returns `true` if `dequeue_work` may block until work is available.
    fn is_dequeue_blocking(&self) -> bool;
    /// Start any background tasks required by the provider.
    async fn start(&self) -> crate::Result<()>;
    /// Stop any background tasks required by the provider.
    async fn stop(&self) -> crate::Result<()>;
}
