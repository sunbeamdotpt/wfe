# WFE

[![Matrix](https://img.shields.io/badge/chat-%23hello%3Asunbeam.pt-0dbd8b?logo=matrix)](https://matrix.to/#/#hello:sunbeam.pt)
[![License](https://img.shields.io/github/license/sunbeamdotpt/wfe)](LICENSE.md)

A persistent, embeddable workflow engine for Rust. Trait-based, pluggable, built for large, highly complex build, test, deployment, and release pipelines of the highest levels of complexity.

---

## What is WFE?

WFE is a technical love letter from a former [VMware](https://www.vmware.com) and [Pivotal](https://pivotal.io) engineer (@siennathesane). Its internal workflow architecture is based on the amazing [workflow-core](https://github.com/danielgerlag/workflow-core) library by Daniel Gerlag, and the YAML structure and support is based on [Concourse CI](https://concourse-ci.org). WFE is a pluggable, extendable library that can be used to design embedded business workflows, CLI applications, and CI/CD pipelines. It is designed to be embedded into your application, and can scale open-ended with pluggable architectures. You can deploy Cloud Foundry, Kubernetes, or even bootstrap a public cloud with WFE.

You can define workflows as code using a fluent builder API, or as YAML files with shell and JavaScript steps. Workflows persist across restarts, support event-driven pausing, parallel execution, saga compensation, and distributed locking. It also comes with native support for containerd and buildkitd embedded into the application.

The only thing not included is a server and a web UI (open to contributions that have been agreed upon and discussed ahead of time).

## Why?

Every CI/CD system [I've](https://src.sunbeam.pt/siennathesane) ever used has been either lovely or terrible. I wanted something that was lovely, but also flexible enough to be used in a variety of contexts. The list of CI systems I've used over the years has been extensive, and they all have their _thing_ that makes them stand out in their own ways. I wanted something that could meet every single requirement I have: a distributed workflow engine that can be embedded into any application, has pluggable executors, a statically-verifiable workflow definition language, a simple, easy to use API, YAML 1.1 merge anchors, YAML 1.2 support, multi-file workflows, can be used as a library, can be used as a CLI, and can be used to write servers.

With that, I wanted the user experience to be essentially identical for embedded application developers, systems engineers, and CI/CD engineers. Whether you write your workflows in code, YAML, or you have written a web server endpoint that accepts gRPC workflow definitions and you're relying on this library as a hosted service, it should feel like the same piece of software in every context.

Hell, I'm so dedicated to this being the most useful, most pragmatic, most embeddable workflow engine library ever that I'm willing to accept a C ABI contribution to make it embeddable in any language.

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

- PostgreSQL 17 on port `5432` (shared workspace instance)
- Valkey 8 on port `6379`
- OpenSearch 2 on port `9200`

SQLite tests use temporary files and run everywhere.

---

## Self-hosting CI pipeline

WFE includes a self-hosting CI pipeline defined in `workflows.yaml` at the repository root. The pipeline uses WFE's own YAML workflow engine to build, test, and publish WFE itself.

### Pipeline architecture

```
                         ci (orchestrator)
                              |
          +-------------------+--------------------+
          |                   |                    |
      preflight            lint               test (fan-out)
     (tool check)     (fmt + clippy)              |
                                       +----------+----------+
                                       |          |          |
                                   test-unit  test-integration  test-containers
                                       |     (docker compose)  (lima VM)
                                       |          |          |
                                       +----------+----------+
                                              |
                                    +---------+---------+
                                    |         |         |
                                  cover    package     tag
                                    |         |         |
                                    +---------+---------+
                                              |
                                    +---------+---------+
                                    |                   |
                                 publish            release
                              (crates.io)        (git tags + notes)
```

### Running the pipeline

```sh
# Default — uses current directory as workspace
cargo run --example run_pipeline -p wfe-yaml -- workflows.yaml

# With explicit configuration
WFE_CONFIG='{"workspace_dir":"/path/to/wfe","registry":"sunbeam","git_remote":"origin","coverage_threshold":85}' \
  cargo run --example run_pipeline -p wfe-yaml -- workflows.yaml
```

### WFE features demonstrated

The pipeline exercises every major WFE feature:

- **Workflow composition** — the `ci` orchestrator invokes child workflows (`lint`, `test`, `cover`, `package`, `tag`, `publish`, `release`) using the `workflow` step type.
- **Shell executor** — most steps run bash commands with configurable timeouts.
- **Deno executor** — the `cover` workflow uses a Deno step to parse coverage JSON; the `release` workflow uses Deno to generate release notes.
- **YAML anchors/templates** — `_templates` defines `shell_defaults` and `long_running` anchors, reused across steps via `<<: *shell_defaults`.
- **Structured outputs** — steps emit `##wfe[output key=value]` markers to pass data between steps and workflows.
- **Variable interpolation** — `((workspace_dir))` syntax passes inputs through workflow composition.
- **Error handling** — `on_failure` handlers, `error_behavior` with retry policies, and `ensure` blocks for cleanup (e.g., `docker-down`, `lima-down`).

### Preflight tool check

The `preflight` workflow runs first and checks for all required tools: `cargo`, `cargo-nextest`, `cargo-llvm-cov`, `docker`, `limactl`, `buildctl`, and `git`. Essential tools (cargo, nextest, git) cause a hard failure if missing. Optional tools (docker, lima, buildctl, llvm-cov) are reported but do not block the pipeline.

### Graceful infrastructure skipping

Integration and container tests handle missing infrastructure without failing:

- **test-integration**: The `docker-up` step checks if Docker is available. If `docker info` fails, it sets `docker_started=false` and exits cleanly. Subsequent steps (`postgres-tests`, `valkey-tests`, `opensearch-tests`) check this flag and skip if Docker is not running.
- **test-containers**: The `lima-up` step checks if `limactl` is installed. If missing, it sets `lima_started=false` and exits cleanly. The `buildkit-tests` and `containerd-tests` steps check this flag and skip accordingly.

This means the pipeline runs successfully on any machine with the essential Rust toolchain, reporting which optional tests were skipped rather than failing outright.

---

## License

[MIT](LICENSE)

Built by [Sunbeam Studios](https://sunbeam.pt). We run this in production. It works.
