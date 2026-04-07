use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use wfe_core::WfeError;
use wfe_core::executor::{StepRegistry, WorkflowExecutor};
use wfe_core::traits::{
    DistributedLockProvider, LifecyclePublisher, PersistenceProvider, QueueProvider, SearchIndex,
    ServiceProvider,
};

use crate::host::WorkflowHost;
use crate::registry::InMemoryWorkflowRegistry;

/// Fluent builder for constructing a `WorkflowHost`.
///
/// Uses the owned-self pattern: each method consumes and returns the builder.
pub struct WorkflowHostBuilder {
    persistence: Option<Arc<dyn PersistenceProvider>>,
    lock_provider: Option<Arc<dyn DistributedLockProvider>>,
    queue_provider: Option<Arc<dyn QueueProvider>>,
    lifecycle: Option<Arc<dyn LifecyclePublisher>>,
    search: Option<Arc<dyn SearchIndex>>,
    log_sink: Option<Arc<dyn wfe_core::traits::LogSink>>,
    service_provider: Option<Arc<dyn ServiceProvider>>,
}

impl WorkflowHostBuilder {
    pub fn new() -> Self {
        Self {
            persistence: None,
            lock_provider: None,
            queue_provider: None,
            lifecycle: None,
            search: None,
            log_sink: None,
            service_provider: None,
        }
    }

    /// Set the persistence provider (required).
    pub fn use_persistence(mut self, persistence: Arc<dyn PersistenceProvider>) -> Self {
        self.persistence = Some(persistence);
        self
    }

    /// Set the distributed lock provider (required).
    pub fn use_lock_provider(mut self, lock_provider: Arc<dyn DistributedLockProvider>) -> Self {
        self.lock_provider = Some(lock_provider);
        self
    }

    /// Set the queue provider (required).
    pub fn use_queue_provider(mut self, queue_provider: Arc<dyn QueueProvider>) -> Self {
        self.queue_provider = Some(queue_provider);
        self
    }

    /// Set an optional lifecycle publisher.
    pub fn use_lifecycle(mut self, lifecycle: Arc<dyn LifecyclePublisher>) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// Set an optional search index.
    pub fn use_search(mut self, search: Arc<dyn SearchIndex>) -> Self {
        self.search = Some(search);
        self
    }

    /// Set an optional log sink for real-time step output streaming.
    pub fn use_log_sink(mut self, sink: Arc<dyn wfe_core::traits::LogSink>) -> Self {
        self.log_sink = Some(sink);
        self
    }

    /// Set an optional service provider for provisioning infrastructure services.
    pub fn use_service_provider(mut self, provider: Arc<dyn ServiceProvider>) -> Self {
        self.service_provider = Some(provider);
        self
    }

    /// Build the `WorkflowHost`.
    ///
    /// Returns an error if persistence, lock_provider, or queue_provider have not been set.
    pub fn build(self) -> wfe_core::Result<WorkflowHost> {
        let persistence = self.persistence.ok_or_else(|| {
            WfeError::Other(
                "PersistenceProvider is required. Call .use_persistence() before .build().".into(),
            )
        })?;
        let lock_provider = self.lock_provider.ok_or_else(|| {
            WfeError::Other(
                "DistributedLockProvider is required. Call .use_lock_provider() before .build()."
                    .into(),
            )
        })?;
        let queue_provider = self.queue_provider.ok_or_else(|| {
            WfeError::Other(
                "QueueProvider is required. Call .use_queue_provider() before .build().".into(),
            )
        })?;

        let mut executor = WorkflowExecutor::new(
            Arc::clone(&persistence),
            Arc::clone(&lock_provider),
            Arc::clone(&queue_provider),
        );

        if let Some(ref lifecycle) = self.lifecycle {
            executor = executor.with_lifecycle(Arc::clone(lifecycle));
        }
        if let Some(ref search) = self.search {
            executor = executor.with_search(Arc::clone(search));
        }
        if let Some(ref log_sink) = self.log_sink {
            executor = executor.with_log_sink(Arc::clone(log_sink));
        }

        Ok(WorkflowHost {
            persistence,
            lock_provider,
            queue_provider,
            lifecycle: self.lifecycle,
            search: self.search,
            service_provider: self.service_provider,
            registry: Arc::new(RwLock::new(InMemoryWorkflowRegistry::new())),
            step_registry: Arc::new(RwLock::new(StepRegistry::new())),
            executor: Arc::new(executor),
            shutdown: CancellationToken::new(),
        })
    }
}

impl Default for WorkflowHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}
