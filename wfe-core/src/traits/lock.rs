use async_trait::async_trait;

/// Distributed lock provider for preventing concurrent execution of the same workflow.
#[async_trait]
pub trait DistributedLockProvider: Send + Sync {
    /// Try to acquire a lock on the given resource. Returns `true` if acquired.
    async fn acquire_lock(&self, resource: &str) -> crate::Result<bool>;
    /// Release a previously acquired lock on the given resource.
    async fn release_lock(&self, resource: &str) -> crate::Result<()>;
    /// Start any background tasks required by the provider.
    async fn start(&self) -> crate::Result<()>;
    /// Stop any background tasks required by the provider.
    async fn stop(&self) -> crate::Result<()>;
}
