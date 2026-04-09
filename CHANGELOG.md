# Changelog

All notable changes to this project will be documented in this file.

## [1.9.1] - 2026-04-09

### Added

- **wfe-kubernetes**: Cross-workflow shared volumes — declare
  `shared_volume: { mount_path: /workspace, size: 30Gi }` on a
  top-level workflow and every sub-workflow in the tree shares a
  single PVC. Sub-workflows inherit the root's namespace via
  `root_workflow_id` so checkout/lint/test/cover all see the same
  `/workspace`.
- **wfe-kubernetes**: Configurable `shell:` on step config (default
  `/bin/sh`). CI workflows that need `set -o pipefail` set
  `shell: /bin/bash` on their shared config anchors.
- **wfe-core**: `StepExecutionContext.definition` gives executor-specific
  steps access to the workflow definition without a registry round-trip.
- Expanded test suites: 14 new persistence tests, 4 queue tests, 3
  lock tests, 10 host name/resolve tests, multi-step K8s integration
  test, 14 webhook handler tests. All shared via macros so every
  provider backend benefits.

### Fixed

- **wfe**: Shared volume config now propagates to sub-workflows via
  `_wfe_shared_volume` in instance data. Previously, only the root
  definition carried the config; sub-workflow steps never saw it and
  no PVC was created.
- **wfe-server**: `log_search` TCP probe used `SocketAddr::parse` which
  rejects hostnames — OpenSearch integration tests were silently
  skipping. Switched to `ToSocketAddrs` for DNS resolution.
- **wfe-server**: `ensure_index` now handles `resource_already_exists`
  races gracefully instead of returning an error.
- **persistence**: `create_new_workflow` falls back to UUID when `name`
  is empty, preventing UNIQUE constraint violations in tests.
- **wfe-kubernetes**: Default shell reverted to `/bin/sh` (was changed
  to `/bin/bash` in 1.9.0 which broke alpine-based containers).

## [1.9.0] - 2026-04-07

### Added

- **wfectl**: New command-line client for wfe-server with 17 subcommands
  (login, logout, whoami, register, validate, definitions, run, get, list,
  cancel, suspend, resume, publish, watch, logs, search-logs). Supports
  OAuth2 PKCE login flow via Ory Hydra, direct bearer-token auth, and
  configurable output formats (table/JSON).
- **wfectl validate**: Local YAML validation command that compiles workflow
  files in-process via `wfe-yaml` with the full executor feature set
  (rustlang, buildkit, containerd, kubernetes, deno). No server round-trip
  or auth required — instant feedback before push.
- **Human-friendly workflow names**: `WorkflowInstance` now has a `name`
  field (unique alongside the UUID primary key). The host auto-assigns
  `{definition_id}-{N}` using a per-definition monotonic counter, with
  optional caller override via `start_workflow_with_name` /
  `StartWorkflowRequest.name`. All gRPC read/mutate APIs accept either the
  UUID or the human name interchangeably. `WorkflowDefinition` now has an
  optional display `name` declared in YAML (e.g. `name: "Continuous
  Integration"`) that surfaces in listings.
- **wfe-server**: Full executor feature set enabled in the shipped binary —
  kubernetes, deno, buildkit, containerd, rustlang step types all compiled
  in.
- **wfe-server**: Dockerfile switched from `rust:alpine` to
  `rust:1-bookworm` + `debian:bookworm-slim` runtime because `deno_core`'s
  bundled v8 only ships glibc binaries.
- **wfe-ci**: New `Dockerfile.ci` builder image with rust stable,
  cargo-nextest, cargo-llvm-cov, sccache, buildctl, kubectl, tea, git.
  Used as the base image for kubernetes-executed CI steps.
- **wfe-kubernetes**: `run:` scripts now execute under `/bin/bash -c`
  instead of `/bin/sh -c` so workflows can rely on `set -o pipefail`,
  process substitution, arrays, and other bashisms dash doesn't support.

### Fixed

- **wfe**: `WorkflowHost::start()` now calls
  `persistence.ensure_store_exists()`, which was previously defined but
  never invoked — the Postgres/SQLite schema was never auto-created on
  startup, causing `relation "wfc.workflows" does not exist` errors on
  first run.
- **wfe-core**: `SubWorkflowStep` now inherits the parent workflow's data
  when no explicit inputs are set, so child workflows see the same
  top-level fields (e.g. `$REPO_URL`, `$COMMIT_SHA`) without every
  `type: workflow` step having to re-declare them.
- **workflows.yaml**: Restructured all step references from
  `<<: *ci_step` / `<<: *ci_long` (which relied on YAML 1.1 shallow merge
  over-writing the `config:` block) to inner-config merges of the form
  `config: {<<: *ci_config, ...}`. Secret env vars moved into the shared
  `ci_env` anchor so individual steps don't fight the shallow merge.

## [1.8.1] - 2026-04-06

### Added

- **wfe-server**: gRPC reflection support via `tonic-reflection`
- **wfe-server**: Schema endpoints: `/schema/workflow.json` (JSON Schema), `/schema/workflow.yaml` (YAML Schema), `/schema/workflow.proto` (raw proto)
- **wfe-yaml**: Auto-generated JSON Schema from `schemars` derives on all YAML types
- **wfe-server**: Dockerfile for multi-stage alpine build with all executor features
- **wfe-server**: Comprehensive configuration reference (README.md)

### Fixed

- **wfe-yaml**: Added missing `license`, `repository`, `homepage` fields to Cargo.toml
- **wfe-buildkit-protos**: Removed vendored Go repos (166MB -> 356K), kept only .proto files
- **wfe-containerd-protos**: Removed vendored Go repos (53MB -> 216K), kept only .proto files
- Filesystem loop warnings from circular symlinks in vendored Go modules eliminated
- Pinned `icu_calendar <2.2` to work around `temporal_rs`/`deno_core` incompatibility

## [1.8.0] - 2026-04-06

### Added

- **wfe-kubernetes**: New crate -- Kubernetes executor for running workflow steps as K8s Jobs
  - Namespace-per-workflow isolation with automatic cleanup
  - Job manifest builder with env, resources, pull policy, node selector support
  - Pod log streaming to LogSink with real-time output capture
  - `##wfe[output key=value]` parsing from Job stdout
  - Timeout handling with Job cleanup on expiry
  - `KubernetesServiceProvider`: provisions infrastructure services as K8s Pods + Services with DNS resolution and readiness polling
  - 100% test coverage on service provider, 91% overall crate coverage
- **wfe-core**: `ServiceDefinition`, `ServicePort`, `ReadinessProbe`, `ServiceEndpoint` types for declaring infrastructure services
- **wfe-core**: `ServiceProvider` trait for pluggable service provisioning
- **wfe-core**: `services` field on `WorkflowDefinition` for declaring required services
- **wfe**: Capability-based workflow routing -- hosts check `can_execute()` before accepting workflows
  - Verifies all step types are registered in the StepRegistry
  - Verifies ServiceProvider is configured and can provision required services
  - Re-queues workflows that can't be handled by this host
- **wfe**: Service lifecycle in dequeue loop -- provision before execution, teardown after completion/failure
- **wfe**: `use_service_provider()` on `WorkflowHostBuilder`
- **wfe-containerd**: `ContainerdServiceProvider` for running services via containerd gRPC API on host network
- **wfe-yaml**: `services:` block in workflow YAML definitions with readiness probes (exec, tcp, http)
- **wfe-yaml**: `kubernetes`/`k8s` step type with lazy client creation

## [1.7.0] - 2026-04-05

### Added

- **wfe-deno**: New crate -- Deno/JS/TS bindings for the WFE workflow engine
  - Full API surface via 23 deno_core ops: host lifecycle, workflow management, fluent builder, step registration, event publishing
  - Channel-based execution bridge: JS step functions called from tokio executor via mpsc/oneshot channels
  - High-level JS API classes: `WorkflowHost`, `WorkflowBuilder`, `ExecutionResult`
  - 52 tests (26 unit + 26 integration), 93% line coverage
- **wfe-core**: `WorkflowBuilder.steps` and `last_step` fields now public

## [1.6.3] - 2026-04-05

### Fixed

- **wfe-core**: Propagate `step_name` into execution pointers when advancing to next steps, compensation steps, and parallel branch children
- **wfe**: Set `step_name` on initial execution pointer when starting a workflow instance

## [1.6.2] - 2026-04-05

### Added

- **wfe-core**: `WorkflowBuilder::add_step_typed()` for adding named, configured steps in parallel branch closures
- **wfe-core**: `WorkflowBuilder::wire_outcome()` now public for custom graph wiring

## [1.6.1] - 2026-04-05

### Added

- **wfe-core**: `StepBuilder::config()` for attaching arbitrary JSON configuration to individual steps, readable at runtime via `context.step.step_config`

## [1.6.0] - 2026-04-01

### Added

- **wfe-server**: Headless workflow server (single binary)
  - gRPC API with 13 RPCs: workflow CRUD, lifecycle streaming, log streaming, log search
  - HTTP webhooks: GitHub and Gitea with HMAC-SHA256 verification, configurable triggers
  - OIDC/JWT authentication with JWKS discovery and asymmetric algorithm allowlist
  - Static bearer token auth with constant-time comparison
  - Lifecycle event broadcasting via `WatchLifecycle` server-streaming RPC
  - Real-time log streaming via `StreamLogs` with follow mode and history replay
  - Full-text log search via OpenSearch with `SearchLogs` RPC
  - Layered config: CLI flags > env vars > TOML file
- **wfe-server-protos**: gRPC service definitions (tonic 0.14, server + client stubs)
- **wfe-core**: `LogSink` trait for real-time step output streaming
- **wfe-core**: Lifecycle publisher wired into executor (StepStarted, StepCompleted, Error, Completed, Terminated)
- **wfe**: `use_log_sink()` on `WorkflowHostBuilder`
- **wfe-yaml**: Shell step streaming mode with `tokio::select!` interleaved stdout/stderr

### Security

- JWT algorithm confusion prevention: derive algorithm from JWK, reject symmetric algorithms
- Constant-time static token comparison via `subtle` crate
- OIDC issuer HTTPS validation to prevent SSRF
- Fail-closed on OIDC discovery failure (server won't start with broken auth)
- Authenticated generic webhook endpoint
- 2MB webhook payload size limit
- Config parse errors fail loudly (no silent fallback to open defaults)
- Blocked sensitive env var injection (PATH, LD_PRELOAD, etc.) from workflow data
- Security regression tests for all critical and high findings

### Fixed

- Shell step streaming path now respects `timeout_ms` with `child.kill()` on timeout
- LogSink properly threaded from WorkflowHostBuilder through executor to StepExecutionContext
- LogStore.with_search() wired in server main.rs for OpenSearch indexing
- OpenSearch `index_chunk` returns Err on HTTP failure instead of swallowing it
- Webhook publish failures return 500 instead of 200

## [1.5.0] - 2026-03-29

### Added

- **wfe-rustlang**: New crate with Rust toolchain step executors
  - Cargo steps: `cargo-build`, `cargo-test`, `cargo-check`, `cargo-clippy`, `cargo-fmt`, `cargo-doc`, `cargo-publish`
  - External tool steps with auto-install: `cargo-audit`, `cargo-deny`, `cargo-nextest`, `cargo-llvm-cov`
  - Rustup steps: `rust-install`, `rustup-toolchain`, `rustup-component`, `rustup-target`
  - `cargo-doc-mdx`: generates MDX documentation from rustdoc JSON output using the `rustdoc-types` crate
- **wfe-yaml**: `rustlang` feature flag enabling all cargo/rustup step types
- **wfe-yaml**: Schema fields for Rust steps (`package`, `features`, `toolchain`, `profile`, `output_dir`, etc.)
- **wfe-containerd**: Remote daemon support via `WFE_IO_DIR` environment variable
- **wfe-containerd**: Image chain ID resolution from content store for proper rootfs snapshots
- **wfe-containerd**: Docker-default Linux capabilities for root containers
- Lima `wfe-test` VM config (Alpine + containerd + BuildKit, TCP socat proxy)
- Containerd integration tests running Rust toolchain in containers

### Fixed

- **wfe-containerd**: Empty rootfs — snapshot parent now resolved from image chain ID instead of empty string
- **wfe-containerd**: FIFO deadlock with remote daemons — replaced with regular file I/O
- **wfe-containerd**: `sh: not found` — use absolute `/bin/sh` path in OCI process spec
- **wfe-containerd**: `setgroups: Operation not permitted` — grant capabilities when running as UID 0

### Changed

- Lima `wfe-test` VM uses Alpine apk packages instead of GitHub release binaries
- Container tests use TCP proxy (`http://127.0.0.1:2500`) instead of Unix socket forwarding
- CI pipeline (`workflows.yaml`) updated with `wfe-rustlang` in test, package, and publish steps

879 tests. 88.8% coverage on wfe-rustlang.

## [1.4.0] - 2026-03-26

### Added

- Type-safe `when:` conditions on workflow steps with compile-time validation
- Full boolean combinator set: `all` (AND), `any` (OR), `none` (NOR), `one_of` (XOR), `not` (NOT)
- Task file includes with cycle detection
- Self-hosting CI pipeline (`workflows.yaml`) demonstrating all features
- `readFile()` op for deno runtime
- Auto-typed `##wfe[output]` annotations (bool, number conversion)
- Multi-workflow YAML files, SubWorkflow step type, typed input/output schemas
- HostContext for programmatic child workflow invocation
- BuildKit image builder and containerd container runner as standalone crates
- gRPC clients generated from official upstream proto files (tonic 0.14)

### Fixed

- Pipeline coverage step produces valid JSON, deno reads it with `readFile()`
- Host context field added to container executor test contexts
- `.outputs.` paths resolved flat for child workflows
- Pointer status conversion for Skipped in postgres provider

629 tests. 87.7% coverage.

## [1.0.0] - 2026-03-23

### Added

- **wfe-core**: Workflow engine with step primitives, executor, fluent builder API
- **wfe**: WorkflowHost, registry, sync runner, and purger
- **wfe-sqlite**: SQLite persistence provider
- **wfe-postgres**: PostgreSQL persistence provider
- **wfe-opensearch**: OpenSearch search index provider
- **wfe-valkey**: Valkey provider for locks, queues, and lifecycle events
- **wfe-yaml**: YAML workflow definitions with shell and deno executors
- **wfe-yaml**: Deno JS/TS runtime with sandboxed permissions, HTTP ops, npm support via esm.sh
- OpenTelemetry tracing support behind `otel` feature flag
- In-memory test support providers
