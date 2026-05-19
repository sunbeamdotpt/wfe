# wfe-yaml

YAML workflow definitions for WFE.

## What it does

`wfe-yaml` lets you define workflows in YAML instead of Rust code. It parses YAML files, validates the schema, interpolates variables, compiles the result into `WorkflowDefinition` + step factories, and hands them to the WFE host. Ships with a shell executor and an optional sandboxed Deno executor for running JavaScript/TypeScript steps.

## Quick start

Define a workflow in YAML:

```yaml
workflow:
  id: deploy
  version: 1
  steps:
    - name: build
      type: shell
      config:
        run: cargo build --release
        timeout: 5m

    - name: test
      type: shell
      config:
        run: cargo test
        env:
          RUST_LOG: info

    - name: notify
      type: deno
      config:
        script: |
          const result = Wfe.getData("build.stdout");
          Wfe.setOutput("status", "deployed");
        permissions:
          net: ["https://hooks.slack.com"]
```

Load and register it:

```rust
use std::collections::HashMap;
use std::path::Path;

let config = HashMap::new();
let compiled = wfe_yaml::load_workflow(Path::new("deploy.yaml"), &config)?;

// Register with the host.
host.register_workflow_definition(compiled.definition).await;
for (key, factory) in compiled.step_factories {
    host.register_step_factory(&key, factory).await;
}
```

### Variable interpolation

Use `${{ var.name }}` syntax in YAML. Variables are resolved from the config map passed to `load_workflow`:

```rust
let mut config = HashMap::new();
config.insert("env".into(), serde_json::json!("production"));

let compiled = wfe_yaml::load_workflow_from_str(&yaml_str, &config)?;
```

## API

| Type | Description |
|---|---|
| `load_workflow()` | Load from a file path with variable interpolation. |
| `load_workflow_from_str()` | Load from a YAML string with variable interpolation. |
| `CompiledWorkflow` | Contains a `WorkflowDefinition` and a `Vec<(String, StepFactory)>` ready for host registration. |
| `YamlWorkflow` | Top-level YAML schema (`workflow:` key). |
| `WorkflowSpec` | Workflow definition: id, version, description, error behavior, steps. |
| `YamlStep` | Step definition: name, type, config, inputs/outputs, parallel children, error handling hooks (`on_success`, `on_failure`, `ensure`). |
| `ShellStep` | Executes shell commands via `tokio::process::Command`. Captures stdout/stderr, parses `##wfe[output name=value]` directives. |
| `DenoStep` | Executes JS/TS in an embedded Deno runtime with configurable permissions. |
| `GitRepoStep` | Clones a git repository with `gix`, stores the working tree as a tarball in the artifact store. |

### Shell step features

- Configurable shell (defaults to `sh`)
- Workflow data injected as `UPPER_CASE` environment variables
- Custom env vars via `config.env`
- Working directory override
- Timeout support
- Structured output via `##wfe[output name=value]` lines in stdout

### Deno step features

- Inline `script` or external `file` source
- Automatic module evaluation for `import`/`export`/`await` syntax
- Granular permissions: `net`, `read`, `write`, `env`, `run`, `dynamic_import`
- `Wfe.getData()` / `Wfe.setOutput()` host bindings
- V8-level timeout enforcement (catches infinite loops)

### Git-repo step features

- Pure-Rust cloning via `gix` — no `git` CLI dependency
- Shallow clones via `config.depth`
- Branch and commit checkout
- Artifact caching: the cloned working tree is tar-gzipped and stored in the artifact store
- Artifact override: skip the clone entirely when an input key already holds an artifact ref

#### Artifact override (recommended for CI)

The `git-repo` step supports an `input` field that references a key in workflow data. If that key contains an artifact ref (`{"__wfe_artifact": "sha256:..."}`), the step skips the network clone and extracts the cached artifact directly:

```yaml
- name: clone
  type: git-repo
  config:
    run: https://github.com/org/repo.git
    branch: main
    input: repo_artifact
    working_dir: /workspace/repo
```

#### Why normal clones don't cache

The artifact store is content-addressed (SHA-256). Two tar archives of the same git checkout usually have different digests because tar captures non-deterministic metadata (mtimes, directory ordering, permissions). This means the store sees every clone as a new artifact. Use the `input` override path when you want to skip clones and rely on a previously stored artifact.

## Features

| Feature | Description |
|---|---|
| `deno` | Enables the Deno JavaScript/TypeScript executor. Pulls in `deno_core`, `deno_error`, `url`, `reqwest`. Off by default. |

## Testing

```sh
# Core tests (shell executor)
cargo test -p wfe-yaml

# With Deno executor
cargo test -p wfe-yaml --features deno
```

No external services required. Deno tests need the `deno` feature flag.

## License

MIT
