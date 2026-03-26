# wfe-core

Core traits, models, builder, and executor for the WFE workflow engine.

## What it does

`wfe-core` defines the foundational abstractions that every other WFE crate builds on. It provides the `StepBody` trait for implementing workflow steps, a fluent `WorkflowBuilder` for composing workflow definitions, a `WorkflowExecutor` that drives step execution with locking and persistence, and a library of built-in control-flow primitives (if/while/foreach/parallel/saga).

## Quick start

Define a step by implementing `StepBody`, then wire steps together with the builder:

```rust
use async_trait::async_trait;
use wfe_core::builder::WorkflowBuilder;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

#[derive(Default)]
struct Greet;

#[async_trait]
impl StepBody for Greet {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        println!("hello from a workflow step");
        Ok(ExecutionResult::next())
    }
}

let definition = WorkflowBuilder::<serde_json::Value>::new()
    .start_with::<Greet>()
    .name("greet")
    .end_workflow()
    .build("hello-workflow", 1);
```

## API

| Type / Trait | Description |
|---|---|
| `StepBody` | The core unit of work. Implement `run()` to define step behavior. |
| `StepExecutionContext` | Runtime context passed to each step (workflow data, persistence data, cancellation token). |
| `WorkflowData` | Marker trait for data flowing between steps. Auto-implemented for anything `Serialize + DeserializeOwned + Send + Sync + Clone`. |
| `WorkflowBuilder<D>` | Fluent builder for composing `WorkflowDefinition`s. Supports `start_with`, `then`, `if_do`, `while_do`, `for_each`, `parallel`, `saga`. |
| `StepBuilder<D>` | Per-step builder returned by `WorkflowBuilder`. Configures name, error behavior, compensation. |
| `WorkflowExecutor` | Acquires a lock, loads the instance, runs all active pointers, processes results, persists. |
| `StepRegistry` | Maps step type names to factory functions. |
| `PersistenceProvider` | Composite trait: `WorkflowRepository + EventRepository + SubscriptionRepository + ScheduledCommandRepository`. |
| `DistributedLockProvider` | Trait for acquiring/releasing workflow-level locks. |
| `QueueProvider` | Trait for enqueuing/dequeuing workflow and event work items. |

### Built-in primitives

| Primitive | Purpose |
|---|---|
| `IfStep` | Conditional branching |
| `WhileStep` | Loop while condition holds |
| `ForEachStep` | Iterate over a collection |
| `SequenceStep` | Parallel branch container |
| `DecideStep` | Multi-way branching |
| `DelayStep` | Pause execution for a duration |
| `ScheduleStep` | Resume at a specific time |
| `WaitForStep` | Suspend until an external event |
| `SagaContainerStep` | Transaction-like compensation |
| `RecurStep` | Recurring/periodic execution |
| `EndStep` | Explicit workflow termination |

## Features

| Feature | Description |
|---|---|
| `test-support` | Exposes `test_support` module with in-memory persistence and helpers for testing workflows. |
| `otel` | Enables OpenTelemetry integration via `tracing-opentelemetry`. |

## Testing

```sh
cargo test -p wfe-core
```

No external dependencies required.

## License

MIT
