use async_trait::async_trait;

use crate::models::QueueType;

/// Queue provider for distributing workflow execution across workers.
#[async_trait]
pub trait QueueProvider: Send + Sync {
    async fn queue_work(&self, id: &str, queue: QueueType) -> crate::Result<()>;
    async fn dequeue_work(&self, queue: QueueType) -> crate::Result<Option<String>>;
    fn is_dequeue_blocking(&self) -> bool;
    async fn start(&self) -> crate::Result<()>;
    async fn stop(&self) -> crate::Result<()>;
}
