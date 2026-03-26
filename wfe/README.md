# wfe

The umbrella crate for the WFE workflow engine.

## What it does

`wfe` provides `WorkflowHost`, the main orchestrator that ties persistence, locking, queuing, and step execution into a single runtime. It re-exports everything from `wfe-core` so downstream code only needs one dependency. Register workflow definitions, start instances, publish events, suspend/resume/terminate -- all through `WorkflowHost`.

## Quick start

```rust
use std::sync::Arc;
use wfe::{WorkflowHostBuilder, WorkflowHost};

// Build a host with your providers.
let host = WorkflowHostBuilder::new()
    .use_persistence(persistence)
    .use_lock_provider(lock_provider)
    .use_queue_provider(queue_provider)
    .build()
    .unwrap();

// Register a workflow definition.
host.register_workflow::<serde_json::Value>(
    &|builder| {
        builder
            .start_with::<MyStep>()
            .name("do the thing")
            .end_workflow()
    },
    "my-workflow",
    1,
).await;

// Register custom steps.
host.register_step::<MyStep>().await;

// Start the background consumers.
host.start().await.unwrap();

// Launch a workflow instance.
let id = host.start_workflow("my-workflow", 1, serde_json::json!({})).await.unwrap();

// Publish an event to resume a waiting workflow.
host.publish_event("approval", "order-123", serde_json::json!({"approved": true})).await.unwrap();
```

### Synchronous runner (testing)

`run_workflow_sync` starts a workflow and polls until it completes or times out. Useful in integration tests.

```rust
use std::time::Duration;
use wfe::run_workflow_sync;

let result = run_workflow_sync(&host, "my-workflow", 1, data, Duration::from_secs(5)).await?;
assert_eq!(result.status, wfe::models::WorkflowStatus::Complete);
```

## API

| Type | Description |
|---|---|
| `WorkflowHost` | Main orchestrator. Registers workflows/steps, starts instances, publishes events, manages lifecycle. |
| `WorkflowHostBuilder` | Fluent builder for `WorkflowHost`. Requires persistence, lock provider, and queue provider. Optionally accepts lifecycle publisher and search index. |
| `InMemoryWorkflowRegistry` | In-memory `WorkflowRegistry` implementation. Stores definitions keyed by `(id, version)`. |
| `run_workflow_sync` | Async helper that starts a workflow and polls until completion or timeout. |
| `purge_workflows` | Cleanup utility for removing completed workflow data from persistence. |

Everything from `wfe-core` is re-exported: `WorkflowBuilder`, `StepBody`, `ExecutionResult`, models, traits, and primitives.

## Features

| Feature | Description |
|---|---|
| `otel` | Enables OpenTelemetry tracing. Pulls in `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, and `tracing-opentelemetry`. Also enables `wfe-core/otel`. |

## Testing

```sh
cargo test -p wfe
```

Tests use `wfe-sqlite` and `wfe-core/test-support` for in-memory persistence. No external services required.

## License

MIT
