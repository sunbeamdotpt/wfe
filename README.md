# WFE

A persistent, embeddable workflow engine for Rust. Trait-based, pluggable, built for real infrastructure.

> Rust port of [workflow-core](https://github.com/danielgerlag/workflow-core), rebuilt from scratch with async/await, pluggable persistence, and a YAML frontend with shell and Deno executors.

---

## What is WFE?

WFE is a workflow engine you embed directly into your Rust application. Define workflows as code using a fluent builder API, or as YAML files with shell and JavaScript steps. Workflows persist across restarts, support event-driven pausing, parallel execution, saga compensation, and distributed locking.

Built for:

- **Persistent workflows** — steps survive process restarts. Pick up where you left off.
- **Embeddable CLIs** — drop it into a binary, no external orchestrator required.
- **Portable CI pipelines** — YAML workflows with shell and Deno steps, variable interpolation, structured outputs.

---

## Architecture

```
wfe/
├── wfe-core          Traits, models, builder, executor, primitives
├── wfe               Umbrella crate — WorkflowHost, WorkflowHostBuilder
├── wfe-yaml          YAML workflow loader, shell executor, Deno executor
├── wfe-sqlite        SQLite persistence + queue + lock provider
├── wfe-postgres      PostgreSQL persistence + queue + lock provider
├── wfe-valkey        Valkey (Redis) distributed lock + queue provider
└── wfe-opensearch    OpenSearch search index provider
```

`wfe-core` defines the traits. Provider crates implement them. `wfe` wires everything together through `WorkflowHost`. `wfe-yaml` adds a YAML frontend with built-in executors.

---

## Quick start — Rust builder API

Define steps by implementing `StepBody`, then chain them with the builder:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wfe::builder::WorkflowBuilder;
use wfe::models::*;
use wfe::traits::step::{StepBody, StepExecutionContext};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct MyData {
    message: String,
}

#[derive(Default)]
struct FetchData;

#[async_trait]
impl StepBody for FetchData {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("Fetching data...");
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct Transform;

#[async_trait]
impl StepBody for Transform {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("Transforming...");
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct Publish;

#[async_trait]
impl StepBody for Publish {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe::Result<ExecutionResult> {
        println!("Publishing.");
        Ok(ExecutionResult::next())
    }
}

let definition = WorkflowBuilder::<MyData>::new()
    .start_with::<FetchData>()
        .name("Fetch")
    .then::<Transform>()
        .name("Transform")
        .on_error(ErrorBehavior::Retry {
            interval: std::time::Duration::from_secs(5),
            max_retries: 3,
        })
    .then::<Publish>()
        .name("Publish")
    .end_workflow()
    .build("etl-pipeline", 1);
```

The builder supports `.then()`, `.parallel()`, `.if_do()`, `.while_do()`, `.for_each()`, `.saga()`, `.compensate_with()`, `.wait_for()`, `.delay()`, and `.then_fn()` for inline closures.

See [`wfe/examples/pizza.rs`](wfe/examples/pizza.rs) for a full example using every feature.

---

## Quick start — YAML

```yaml
workflow:
  id: deploy-pipeline
  version: 1
  steps:
    - name: Lint
      config:
        run: cargo clippy --all-targets -- -D warnings
        timeout: "120s"

    - name: Test
      config:
        run: cargo test --workspace
        timeout: "300s"

    - name: Build
      config:
        run: cargo build --release
        timeout: "600s"

    - name: Notify
      type: deno
      config:
        script: |
          const result = await fetch("https://hooks.slack.com/...", {
            method: "POST",
            body: JSON.stringify({ text: "Deploy complete" }),
          });
          Wfe.setOutput("status", result.status.toString());
        permissions:
          net: ["hooks.slack.com"]
        timeout: "10s"
```

Load and run:

```rust
use std::collections::HashMap;
use std::path::Path;

let config = HashMap::new();
let compiled = wfe_yaml::load_workflow(Path::new("deploy.yaml"), &config)?;
```

Variables use `${{ var.name }}` interpolation syntax. Outputs from earlier steps are available as workflow data in later steps.

---

## Providers

| Concern | Provider | Crate | Connection |
|---------|----------|-------|------------|
| Persistence | SQLite | `wfe-sqlite` | File path or `:memory:` |
| Persistence | PostgreSQL | `wfe-postgres` | `postgres://user:pass@host/db` |
| Distributed lock | Valkey / Redis | `wfe-valkey` | `redis://host:6379` |
| Queue | Valkey / Redis | `wfe-valkey` | Same connection |
| Search index | OpenSearch | `wfe-opensearch` | `http://host:9200` |

All providers implement traits from `wfe-core`. SQLite and PostgreSQL crates include their own lock and queue implementations for single-node deployments. Use Valkey when you need distributed coordination across multiple hosts.

In-memory implementations of every trait ship with `wfe-core` (behind the `test-support` feature) for testing and prototyping.

---

## The Deno executor

The `deno` step type embeds a V8 runtime for running JavaScript or TypeScript inside your workflow. Scripts run in a sandboxed environment with fine-grained permissions.

```yaml
- name: Process webhook
  type: deno
  config:
    script: |
      const data = Wfe.getData();
      const response = await fetch(`https://api.example.com/v1/${data.id}`);
      const result = await response.json();
      Wfe.setOutput("processed", JSON.stringify(result));
    permissions:
      net: ["api.example.com"]
      read: []
      write: []
      env: []
      run: false
    timeout: "30s"
```

| Permission | Type | Default | What it controls |
|------------|------|---------|-----------------|
| `net` | `string[]` | `[]` | Allowed network hosts |
| `read` | `string[]` | `[]` | Allowed filesystem read paths |
| `write` | `string[]` | `[]` | Allowed filesystem write paths |
| `env` | `string[]` | `[]` | Allowed environment variable names |
| `run` | `bool` | `false` | Whether subprocess spawning is allowed |
| `dynamic_import` | `bool` | `false` | Whether dynamic `import()` is allowed |

Everything is denied by default. You allowlist what each step needs. The V8 isolate is terminated hard on timeout — no infinite loops surviving on your watch.

Enable with the `deno` feature flag on `wfe-yaml`.

---

## Feature flags

| Crate | Flag | What it enables |
|-------|------|-----------------|
| `wfe` | `otel` | OpenTelemetry tracing (spans for every step execution) |
| `wfe-core` | `otel` | OTel span attributes on the executor |
| `wfe-core` | `test-support` | In-memory persistence, lock, and queue providers |
| `wfe-yaml` | `deno` | Deno JavaScript/TypeScript executor |

---

## Testing

Unit tests run without any external dependencies:

```sh
cargo test --workspace
```

Integration tests for PostgreSQL, Valkey, and OpenSearch need their backing services. A Docker Compose file is included:

```sh
docker compose up -d
cargo test --workspace
docker compose down
```

The compose file starts:

- PostgreSQL 17 on port `5433`
- Valkey 8 on port `6379`
- OpenSearch 2 on port `9200`

SQLite tests use temporary files and run everywhere.

---

## License

[MIT](LICENSE)

Built by [Sunbeam Studios](https://sunbeam.pt). We run this in production. It works.
