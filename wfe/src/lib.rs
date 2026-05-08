#![warn(missing_docs)]
//! wfe — Umbrella crate for the WFE workflow engine. Re-exports wfe-core and
//! provides WorkflowHost and WorkflowHostBuilder.
/// Host.
pub mod host;
/// Host builder.
pub mod host_builder;
/// Purger.
pub mod purger;
/// Registry.
pub mod registry;
/// Sync runner.
pub mod sync_runner;

// Re-export everything useful from wfe-core.
pub use wfe_core::*;

// Re-export the new types at top level for convenience.
pub use host::WorkflowHost;
pub use host_builder::WorkflowHostBuilder;
pub use purger::purge_workflows;
pub use registry::InMemoryWorkflowRegistry;
pub use sync_runner::run_workflow_sync;
