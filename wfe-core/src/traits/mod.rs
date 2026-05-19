//! Core traits that define the WFE plugin architecture.
//!
//! Implement these traits to provide persistence, locking, queuing, lifecycle
//! events, search, logging, and service provisioning.
//!
//! | Trait | Purpose | Example Implementations |
//! |-------|---------|------------------------|
//! | [`StepBody`](crate::traits::step::StepBody) | Define a workflow step | Your own types |
//! | [`WorkflowData`](crate::traits::step::WorkflowData) | Data flowing between steps | Any serializable type |
//! | [`HostContext`](crate::traits::step::HostContext) | Start child workflows from a step | `WorkflowHost` |
//! | [`PersistenceProvider`](crate::traits::persistence::PersistenceProvider) | Store workflow state | `wfe-sqlite`, `wfe-postgres` |
//! | [`DistributedLockProvider`](crate::traits::lock::DistributedLockProvider) | Prevent concurrent execution | `wfe-sqlite`, `wfe-valkey` |
//! | [`QueueProvider`](crate::traits::queue::QueueProvider) | Enqueue/dequeue work | `wfe-sqlite`, `wfe-valkey` |
//! | [`LifecyclePublisher`](crate::traits::lifecycle::LifecyclePublisher) | Broadcast status changes | `wfe-valkey` |
//! | [`SearchIndex`](crate::traits::search::SearchIndex) | Full-text search | `wfe-opensearch` |
//! | [`LogSink`](crate::traits::log_sink::LogSink) | Stream step output | Custom webhook sink |
//! | [`ServiceProvider`](crate::traits::service::ServiceProvider) | Provision infra | `wfe-kubernetes` |
//! | [`WorkflowRegistry`](crate::traits::registry::WorkflowRegistry) | Store definitions | `InMemoryWorkflowRegistry` |
//! | [`ArtifactStore`](crate::traits::artifact_store::ArtifactStore) | Content-addressed artifact storage | `LocalArtifactStore` |

/// Content-addressed storage for OCI-compatible artifact blobs.
pub mod artifact_store;
/// Broadcast workflow lifecycle events (started, completed, failed, etc.).
pub mod lifecycle;
/// Distributed locking for workflow instances.
pub mod lock;
/// Real-time step output streaming.
pub mod log_sink;
/// Step and workflow middleware hooks.
pub mod middleware;
/// Persistence for workflow state, events, and subscriptions.
pub mod persistence;
/// Work queue for workflow and event consumers.
pub mod queue;
/// Workflow definition registry.
pub mod registry;
/// Full-text search over workflow instances.
pub mod search;
/// Infrastructure service provisioning.
pub mod service;
/// The core step trait and execution context.
pub mod step;

pub use lifecycle::LifecyclePublisher;
pub use lock::DistributedLockProvider;
pub use log_sink::{LogChunk, LogSink, LogStreamType};
pub use middleware::{StepMiddleware, WorkflowMiddleware};
pub use persistence::{
    EventRepository, PersistenceProvider, ScheduledCommandRepository, SubscriptionRepository,
    WorkflowRepository,
};
pub use queue::QueueProvider;
pub use registry::WorkflowRegistry;
pub use search::{Page, SearchFilter, SearchIndex, WorkflowSearchResult};
pub use service::ServiceProvider;
pub use step::{HostContext, StepBody, StepExecutionContext, WorkflowData};
pub use artifact_store::ArtifactStore;
