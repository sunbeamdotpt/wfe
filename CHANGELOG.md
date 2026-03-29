# Changelog

All notable changes to this project will be documented in this file.

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
