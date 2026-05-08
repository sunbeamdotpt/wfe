#![warn(missing_docs)]
//! wfe-core — Core traits, models, builder, executor, and primitives for the WFE
//! persistent workflow engine.
//!
//! # What is WFE?
//!
//! WFE (Workflow Engine) is a trait-based, pluggable workflow engine for Rust.
//! It is designed for long-running, persistent workflows that survive process
//! restarts. You define workflows as code using a fluent builder API, and the
//! executor drives them to completion with support for parallel branches,
//! conditional logic, loops, saga compensation, and event-driven pausing.
//!
//! # Core concepts
//!
//! | Concept | Description |
//! |---------|-------------|
//! | [`StepBody`](crate::traits::step::StepBody) | The trait you implement to define a unit of work. |
//! | [`WorkflowData`](crate::traits::step::WorkflowData) | The data type that flows between steps. Must be serializable. |
//! | [`WorkflowBuilder`](crate::builder::WorkflowBuilder) | Fluent API for composing workflow definitions. |
//! | [`StepBuilder`](crate::builder::StepBuilder) | Per-step configuration (name, error handling, compensation). |
//! | [`WorkflowExecutor`](crate::executor::WorkflowExecutor) | Drives execution: acquires locks, runs steps, persists state. |
//! | [`StepExecutionContext`](crate::traits::step::StepExecutionContext) | Runtime context passed to each step (data, pointers, tokens). |
//! | [`ExecutionResult`](crate::models::ExecutionResult) | What a step returns to control flow (`next`, `branch`, `sleep`, etc.). |
//! | [`WorkflowDefinition`](crate::models::WorkflowDefinition) | The compiled, serializable blueprint of a workflow. |
//! | [`WorkflowInstance`](crate::models::WorkflowInstance) | A running (or persisted) execution of a definition. |
//! | [`ExecutionPointer`](crate::models::ExecutionPointer) | Tracks the position of a single branch of execution. |
//!
//! # Hello workflow
//!
//! ```ignore
//! use async_trait::async_trait;
//! use serde::{Deserialize, Serialize};
//! use wfe_core::builder::WorkflowBuilder;
//! use wfe_core::models::ExecutionResult;
//! use wfe_core::traits::step::{StepBody, StepExecutionContext};
//!
//! #[derive(Debug, Clone, Default, Serialize, Deserialize)]
//! struct OrderData {
//!     order_id: String,
//!     amount: f64,
//! }
//!
//! #[derive(Default)]
//! struct ValidateOrder;
//!
//! #[async_trait]
//! impl StepBody for ValidateOrder {
//!     async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
//!         let data: OrderData = ctx.workflow.data()?;
//!         if data.amount <= 0.0 {
//!             return Err(wfe_core::WfeError::Execution("amount must be positive".into()));
//!         }
//!         Ok(ExecutionResult::next())
//!     }
//! }
//!
//! #[derive(Default)]
//! struct ProcessPayment;
//!
//! #[async_trait]
//! impl StepBody for ProcessPayment {
//!     async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
//!         println!("Processing payment...");
//!         Ok(ExecutionResult::next())
//!     }
//! }
//!
//! let definition = WorkflowBuilder::<OrderData>::new()
//!     .start_with::<ValidateOrder>()
//!         .name("Validate")
//!     .then::<ProcessPayment>()
//!         .name("Payment")
//!     .end_workflow()
//!     .build("order-pipeline", 1);
//! ```
//!
//! # Builder patterns
//!
//! The builder supports linear chains, containers, and control flow:
//!
//! - `.then::<S>()` — sequential next step
//! - `.parallel(|b| { b.add_step_typed::<A>("a", None); b.add_step_typed::<B>("b", None); })` — run branches in parallel
//! - `.if_do(|b| { b.add_step_typed::<Then>("then", None); })` — conditional branch
//! - `.while_do(|b| { b.add_step_typed::<LoopBody>("loop", None); })` — loop while condition holds
//! - `.for_each("items", |b| { b.add_step_typed::<ProcessItem>("item", None); })` — iterate over a collection
//! - `.saga(|b| { b.add_step_typed::<Do>("do", None); }).compensate_with::<Undo>()` — saga with compensation
//! - `.wait_for("event_name", "event_key")` — suspend until an external event arrives
//! - `.delay(Duration::from_secs(30))` — pause for a fixed duration
//! - `.then_fn(|| { println!("inline"); ExecutionResult::next() })` — inline closure step
//!
//! # Execution model
//!
//! 1. You build a [`WorkflowDefinition`](crate::models::WorkflowDefinition) using the builder API.
//! 2. You register the definition and all step types with a `WorkflowHost` (from the `wfe` crate).
//! 3. The host creates a [`WorkflowInstance`](crate::models::WorkflowInstance), persists it, and queues it for execution.
//! 4. The [`WorkflowExecutor`](crate::executor::WorkflowExecutor) picks up the instance, acquires a distributed lock, and runs each active [`ExecutionPointer`](crate::models::ExecutionPointer).
//! 5. After each step, the executor processes the [`ExecutionResult`](crate::models::ExecutionResult) — branching, suspending, or completing pointers — and persists the new state.
//! 6. The lock is released, and the instance is re-queued if there is more work to do.
//!
//! This means workflows are **durable**: if the process crashes after step 3, the next executor invocation will resume from step 4 because the pointer state was persisted.
//!
//! # Primitives
//!
//! Built-in control-flow steps live in [`primitives`]:
//!
//! | Primitive | Purpose |
//! |-----------|---------|
//! | [`IfStep`](primitives::if_step::IfStep) | Conditional branching with `then`/`else` children |
//! | [`WhileStep`](primitives::while_step::WhileStep) | Loop while a condition evaluates to `true` |
//! | [`ForEachStep`](primitives::foreach_step::ForEachStep) | Iterate over a JSON array in workflow data |
//! | [`SequenceStep`](primitives::sequence::SequenceStep) | Parallel branch container (all children run concurrently) |
//! | [`DecideStep`](primitives::decide::DecideStep) | Multi-way branch (switch/case style) |
//! | [`DelayStep`](primitives::delay::DelayStep) | Pause execution for a duration |
//! | [`ScheduleStep`](primitives::schedule::ScheduleStep) | Resume at a specific wall-clock time |
//! | [`WaitForStep`](primitives::wait_for::WaitForStep) | Suspend until an external event is published |
//! | [`SagaContainerStep`](primitives::saga_container::SagaContainerStep) | Transaction-like container with compensation on failure |
//! | [`RecurStep`](primitives::recur::RecurStep) | Recurring/periodic execution |
//! | [`PollEndpointStep`](primitives::poll_endpoint::PollEndpointStep) | Poll an HTTP endpoint until a condition is met |
//! | [`SubWorkflowStep`](primitives::sub_workflow::SubWorkflowStep) | Start a child workflow and wait for it to complete |
//! | [`EndStep`](primitives::end_step::EndStep) | Explicit workflow termination |
//!
//! # Error handling
//!
//! Steps return [`ExecutionResult`](crate::models::ExecutionResult) which can signal:
//! - [`ExecutionResult::next()`](crate::models::ExecutionResult::next) — continue to the next step
//! - [`ExecutionResult::outcome(v)`](crate::models::ExecutionResult::outcome) — follow a named outcome branch
//! - [`ExecutionResult::sleep(d)`](crate::models::ExecutionResult::sleep) — pause execution (used by `DelayStep`)
//! - `Err(WfeError::Execution(...))` — mark the pointer as failed
//! - [`ExecutionResult::persist(d)`](crate::models::ExecutionResult::persist) — mark the pointer as complete
//!
//! When a step fails, the executor checks the step's [`ErrorBehavior`](models::ErrorBehavior):
//! - `Retry { interval, max_retries }` — retry with backoff
//! - `Suspend` — pause the workflow for manual intervention
//! - `CompensateThenRetry` — run the compensation step, then retry
//! - `CompensateThenSuspend` — run compensation, then suspend
//!
//! # Feature flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `test-support` | In-memory persistence, lock, and queue providers for unit testing |
//! | `otel` | OpenTelemetry tracing integration |
//!
//! # Testing
//!
//! ```sh
//! cargo test -p wfe-core
//! ```
//!
//! No external dependencies required.

/// Fluent builder API for composing workflow definitions.
///
/// See [`WorkflowBuilder`](crate::builder::WorkflowBuilder) for the main entry point and
/// [`StepBuilder`](crate::builder::StepBuilder) for per-step configuration.
pub mod builder;

/// Error types and the [`Result`](error::Result) alias used throughout WFE.
pub mod error;

/// The workflow executor and supporting infrastructure.
///
/// [`WorkflowExecutor`](executor::WorkflowExecutor) is the heart of the engine.
/// It acquires locks, loads instances, runs active pointers, and persists state.
pub mod executor;

/// Data models for workflows, instances, pointers, events, and execution results.
pub mod models;

/// Built-in control-flow primitives (if, while, foreach, saga, etc.).
pub mod primitives;

/// Core traits that define the plugin architecture.
///
/// Implement these traits to provide persistence, locking, queuing, lifecycle
/// events, search, logging, and service provisioning.
pub mod traits;

#[cfg(any(test, feature = "test-support"))]
/// In-memory test doubles for every provider trait.
pub mod test_support;

pub use error::{Result, WfeError};
