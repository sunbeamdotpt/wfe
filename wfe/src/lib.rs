#![warn(missing_docs)]
//! wfe — Umbrella crate for the WFE workflow engine.
//!
//! This crate re-exports everything from `wfe-core` and adds [`WorkflowHost`],
//! the main orchestrator that ties persistence, locking, queuing, and execution
//! into a single runtime.
//!
//! # Getting started
//!
//! A minimal workflow host needs three providers: persistence, locking, and queuing.
//! For local development or testing, use the in-memory providers from
//! `wfe-core::test_support`, or use `wfe-sqlite` for file-based persistence.
//!
//! ```ignore
//! use std::sync::Arc;
//! use wfe::{WorkflowHostBuilder, WorkflowHost};
//! use wfe_core::test_support::{InMemoryPersistenceProvider, InMemoryLockProvider, InMemoryQueueProvider};
//!
//! # #[tokio::main]
//! # async fn main() -> wfe::Result<()> {
//! let host = WorkflowHostBuilder::new()
//!     .use_persistence(Arc::new(InMemoryPersistence::new()))
//!     .use_lock_provider(Arc::new(InMemoryLockProvider::new()))
//!     .use_queue_provider(Arc::new(InMemoryQueueProvider::new()))
//!     .build()?;
//!
//! // Register a workflow definition built with the fluent API.
//! host.register_workflow::<serde_json::Value>(
//!     &|builder| {
//!         builder
//!             .start_with::<MyStep>()
//!             .name("do the thing")
//!             .end_workflow()
//!     },
//!     "my-workflow",
//!     1,
//! ).await;
//!
//! // Register the step type so the executor can construct it.
//! host.register_step::<MyStep>().await;
//!
//! // Start background consumer tasks.
//! host.start().await?;
//!
//! // Launch an instance.
//! let id = host.start_workflow("my-workflow", 1, serde_json::json!({"key": "value"})).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │              WorkflowHost                   │
//! │  ┌─────────┐  ┌─────────┐  ┌────────────┐ │
//! │  │Registry │  │Executor │  │ Providers  │ │
//! │  │(defs)   │  │(run)    │  │(persist)   │ │
//! │  └────┬────┘  └────┬────┘  └─────┬──────┘ │
//! │       │            │             │        │
//! │  ┌────┴────────────┴─────────────┴──────┐ │
//! │  │         Background Consumers          │ │
//! │  │   (workflow queue + event queue)      │ │
//! │  └───────────────────────────────────────┘ │
//! └─────────────────────────────────────────────┘
//! ```
//!
//! # Lifecycle
//!
//! 1. **Build** the host with [`WorkflowHostBuilder`].
//! 2. **Register** workflow definitions with [`WorkflowHost::register_workflow`].
//! 3. **Register** step types with [`WorkflowHost::register_step`].
//! 4. **Start** the host with [`WorkflowHost::start`] to spawn background consumers.
//! 5. **Launch** instances with [`WorkflowHost::start_workflow`].
//! 6. **Publish events** with [`WorkflowHost::publish_event`] to resume waiting workflows.
//! 7. **Query / control** instances with [`WorkflowHost::get_workflow`],
//!    [`WorkflowHost::suspend_workflow`], [`WorkflowHost::resume_workflow`],
//!    [`WorkflowHost::terminate_workflow`].
//!
//! # Re-exports
//!
//! Everything from `wfe-core` is re-exported at the top level for convenience:
//! [`WorkflowBuilder`](crate::builder::WorkflowBuilder),
//! [`StepBody`](crate::traits::step::StepBody),
//! [`ExecutionResult`](crate::models::ExecutionResult), models, traits, and primitives.
//!
//! # Feature flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `otel` | OpenTelemetry tracing integration |

/// The main orchestrator that ties all workflow engine components together.
///
/// See [`WorkflowHost`](host::WorkflowHost) for the primary API.
pub mod host;

/// Fluent builder for constructing a [`WorkflowHost`](host::WorkflowHost).
///
/// Requires persistence, lock provider, and queue provider. All other
/// providers are optional.
pub mod host_builder;

/// Workflow cleanup utilities.
pub mod purger;

/// In-memory workflow definition registry.
pub mod registry;

/// Synchronous blocking runner for integration tests.
pub mod sync_runner;

// Re-export everything useful from wfe-core.
pub use wfe_core::*;

// Re-export the new types at top level for convenience.
pub use host::WorkflowHost;
pub use host_builder::WorkflowHostBuilder;
pub use purger::purge_workflows;
pub use registry::InMemoryWorkflowRegistry;
pub use sync_runner::run_workflow_sync;
