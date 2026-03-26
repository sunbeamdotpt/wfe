# wfe-buildkit

BuildKit image builder executor for WFE.

## What it does

`wfe-buildkit` provides a `BuildkitStep` that implements the `StepBody` trait from `wfe-core`. It shells out to the `buildctl` CLI to build container images using BuildKit, capturing stdout/stderr and parsing image digests from the output.

## Quick start

Use it standalone:

```rust
use wfe_buildkit::{BuildkitConfig, BuildkitStep};

let config = BuildkitConfig {
    dockerfile: "Dockerfile".to_string(),
    context: ".".to_string(),
    tags: vec!["myapp:latest".to_string()],
    push: true,
    ..Default::default()
};

let step = BuildkitStep::new(config);

// Inspect the command that would be executed.
let args = step.build_command();
println!("{}", args.join(" "));
```

Or use it through `wfe-yaml` with the `buildkit` feature:

```yaml
workflow:
  id: build-image
  version: 1
  steps:
    - name: build
      type: buildkit
      config:
        dockerfile: Dockerfile
        context: .
        tags:
          - myapp:latest
          - myapp:v1.0
        push: true
        build_args:
          RUST_VERSION: "1.78"
        cache_from:
          - type=registry,ref=myapp:cache
        cache_to:
          - type=registry,ref=myapp:cache,mode=max
        timeout: 10m
```

## Configuration

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `dockerfile` | String | Yes | - | Path to the Dockerfile |
| `context` | String | Yes | - | Build context directory |
| `target` | String | No | - | Multi-stage build target |
| `tags` | Vec\<String\> | No | [] | Image tags |
| `build_args` | Map\<String, String\> | No | {} | Build arguments |
| `cache_from` | Vec\<String\> | No | [] | Cache import sources |
| `cache_to` | Vec\<String\> | No | [] | Cache export destinations |
| `push` | bool | No | false | Push image after build |
| `output_type` | String | No | "image" | Output type: image, local, tar |
| `buildkit_addr` | String | No | unix:///run/buildkit/buildkitd.sock | BuildKit daemon address |
| `tls` | TlsConfig | No | - | TLS certificate paths |
| `registry_auth` | Map\<String, RegistryAuth\> | No | {} | Registry credentials |
| `timeout_ms` | u64 | No | - | Execution timeout in milliseconds |

## Output data

After execution, the step writes the following keys into `output_data`:

| Key | Description |
|---|---|
| `{step_name}.digest` | Image digest (sha256:...), if found in output |
| `{step_name}.tags` | Array of tags applied to the image |
| `{step_name}.stdout` | Full stdout from buildctl |
| `{step_name}.stderr` | Full stderr from buildctl |

## Testing

```sh
cargo test -p wfe-buildkit
```

The `build_command()` method returns the full argument list without executing, making it possible to test command construction without a running BuildKit daemon.

## License

MIT
