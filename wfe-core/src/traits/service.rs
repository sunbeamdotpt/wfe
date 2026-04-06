use async_trait::async_trait;

use crate::models::service::{ServiceDefinition, ServiceEndpoint};

/// Provides infrastructure services (databases, caches, etc.) for workflows.
///
/// Implementations handle provisioning, readiness checking, and teardown
/// of services on a specific platform (Kubernetes, containerd, etc.).
#[async_trait]
pub trait ServiceProvider: Send + Sync {
    /// Check if this provider can provision the given services.
    ///
    /// Returns false if any service cannot be handled by this provider.
    fn can_provision(&self, services: &[ServiceDefinition]) -> bool;

    /// Provision all services for a workflow.
    ///
    /// Creates the service containers/pods, waits for them to be ready,
    /// and returns endpoint information for each service.
    async fn provision(
        &self,
        workflow_id: &str,
        services: &[ServiceDefinition],
    ) -> crate::Result<Vec<ServiceEndpoint>>;

    /// Tear down all services for a workflow.
    ///
    /// Called on workflow completion, failure, or termination.
    /// Should be idempotent -- safe to call multiple times.
    async fn teardown(&self, workflow_id: &str) -> crate::Result<()>;
}
