# wfe-containerd

Containerd container runner executor for WFE.

## What it does

`wfe-containerd` runs containers via `nerdctl` as workflow steps. It pulls images, manages registry authentication, and executes containers with configurable networking, resource limits, volume mounts, and TLS settings. Output is captured and parsed for `##wfe[output key=value]` directives, following the same convention as the shell executor.

## Quick start

Add a containerd step to your YAML workflow:

```yaml
workflow:
  id: container-pipeline
  version: 1
  steps:
    - name: run-tests
      type: containerd
      config:
        image: node:20-alpine
        run: npm test
        network: none
        memory: 512m
        cpu: "1.0"
        timeout: 5m
        env:
          NODE_ENV: test
        volumes:
          - source: /workspace
            target: /app
            readonly: true
```

Enable the feature in `wfe-yaml`:

```toml
[dependencies]
wfe-yaml = { version = "1.0.0", features = ["containerd"] }
```

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `image` | `String` | required | Container image to run |
| `run` | `String` | - | Shell command (uses `sh -c`) |
| `command` | `Vec<String>` | - | Command array (mutually exclusive with `run`) |
| `env` | `HashMap` | `{}` | Environment variables |
| `volumes` | `Vec<VolumeMount>` | `[]` | Volume mounts |
| `working_dir` | `String` | - | Working directory inside container |
| `user` | `String` | `65534:65534` | User/group to run as (nobody by default) |
| `network` | `String` | `none` | Network mode: `none`, `host`, or `bridge` |
| `memory` | `String` | - | Memory limit (e.g. `512m`, `1g`) |
| `cpu` | `String` | - | CPU limit (e.g. `1.0`, `0.5`) |
| `pull` | `String` | `if-not-present` | Pull policy: `always`, `if-not-present`, `never` |
| `containerd_addr` | `String` | `/run/containerd/containerd.sock` | Containerd socket address |
| `tls` | `TlsConfig` | - | TLS configuration for containerd connection |
| `registry_auth` | `HashMap` | `{}` | Registry authentication per registry hostname |
| `timeout` | `String` | - | Execution timeout (e.g. `30s`, `5m`) |

## Output parsing

The step captures stdout and stderr. Lines matching `##wfe[output key=value]` are extracted as workflow outputs. Raw stdout, stderr, and exit code are also available under `{step_name}.stdout`, `{step_name}.stderr`, and `{step_name}.exit_code`.

## Security defaults

- Runs as nobody (`65534:65534`) by default
- Network disabled (`none`) by default
- Containers are always `--rm` (removed after execution)
