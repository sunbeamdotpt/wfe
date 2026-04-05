# Changelog

All notable changes to this project will be documented in this file.

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
