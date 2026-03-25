use async_trait::async_trait;

/// Distributed lock provider for preventing concurrent execution of the same workflow.
#[async_trait]
pub trait DistributedLockProvider: Send + Sync {
    async fn acquire_lock(&self, resource: &str) -> crate::Result<bool>;
    async fn release_lock(&self, resource: &str) -> crate::Result<()>;
    async fn start(&self) -> crate::Result<()>;
    async fn stop(&self) -> crate::Result<()>;
}
