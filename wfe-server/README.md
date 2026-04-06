# wfe-server

Headless workflow server with gRPC API, HTTP webhooks, and OIDC authentication.

## Quick Start

```bash
# Minimal (SQLite + in-memory queue)
wfe-server

# Production (Postgres + Valkey + OpenSearch + OIDC)
wfe-server \
  --db-url postgres://wfe:secret@postgres:5432/wfe \
  --queue valkey --queue-url redis://valkey:6379 \
  --search-url http://opensearch:9200
```

## Docker

```bash
docker build -t wfe-server .
docker run -p 50051:50051 -p 8080:8080 wfe-server
```

## Configuration

Configuration is layered: **CLI flags > environment variables > TOML config file**.

### CLI Flags / Environment Variables

| Flag | Env Var | Default | Description |
|------|---------|---------|-------------|
| `--config` | - | `wfe-server.toml` | Path to TOML config file |
| `--grpc-addr` | `WFE_GRPC_ADDR` | `0.0.0.0:50051` | gRPC listen address |
| `--http-addr` | `WFE_HTTP_ADDR` | `0.0.0.0:8080` | HTTP listen address (webhooks) |
| `--persistence` | `WFE_PERSISTENCE` | `sqlite` | Persistence backend: `sqlite` or `postgres` |
| `--db-url` | `WFE_DB_URL` | `wfe.db` | Database URL or file path |
| `--queue` | `WFE_QUEUE` | `memory` | Queue backend: `memory` or `valkey` |
| `--queue-url` | `WFE_QUEUE_URL` | `redis://127.0.0.1:6379` | Valkey/Redis URL |
| `--search-url` | `WFE_SEARCH_URL` | *(none)* | OpenSearch URL (enables search) |
| `--workflows-dir` | `WFE_WORKFLOWS_DIR` | *(none)* | Directory to auto-load YAML workflows |
| `--auth-tokens` | `WFE_AUTH_TOKENS` | *(none)* | Comma-separated static bearer tokens |

### TOML Config File

```toml
# Network
grpc_addr = "0.0.0.0:50051"
http_addr = "0.0.0.0:8080"

# Auto-load workflow definitions from this directory
workflows_dir = "/etc/wfe/workflows"

# --- Persistence ---
[persistence]
backend = "postgres"                          # "sqlite" or "postgres"
url = "postgres://wfe:secret@postgres:5432/wfe"
# For SQLite:
# backend = "sqlite"
# path = "/data/wfe.db"

# --- Queue / Locking ---
[queue]
backend = "valkey"                            # "memory" or "valkey"
url = "redis://valkey:6379"

# --- Search ---
[search]
url = "http://opensearch:9200"                # Enables workflow + log search

# --- Authentication ---
[auth]
# Static bearer tokens (simple API auth)
tokens = ["my-secret-token"]

# OIDC/JWT authentication (e.g., Ory Hydra, Keycloak, Auth0)
oidc_issuer = "https://auth.sunbeam.pt/"
oidc_audience = "wfe-server"                  # Expected 'aud' claim

# Webhook HMAC secrets (per source)
[auth.webhook_secrets]
github = "whsec_github_secret_here"
gitea = "whsec_gitea_secret_here"

# --- Webhooks ---

# Each trigger maps an incoming webhook event to a workflow.
[[webhook.triggers]]
source = "github"                             # "github" or "gitea"
event = "push"                                # GitHub/Gitea event type
match_ref = "refs/heads/main"                 # Optional: only trigger on this ref
workflow_id = "ci"                            # Workflow definition to start
version = 1

[webhook.triggers.data_mapping]
repo = "$.repository.full_name"               # JSONPath from webhook payload
commit = "$.head_commit.id"
branch = "$.ref"

[[webhook.triggers]]
source = "gitea"
event = "push"
workflow_id = "deploy"
version = 1
```

## Persistence Backends

### SQLite

Single-file embedded database. Good for development and single-node deployments.

```toml
[persistence]
backend = "sqlite"
path = "/data/wfe.db"
```

### PostgreSQL

Production-grade. Required for multi-node deployments.

```toml
[persistence]
backend = "postgres"
url = "postgres://user:password@host:5432/dbname"
```

The server runs migrations automatically on startup.

## Queue Backends

### In-Memory

Default. Single-process only -- workflows are lost on restart.

```toml
[queue]
backend = "memory"
```

### Valkey / Redis

Production-grade distributed queue and locking. Required for multi-node.

```toml
[queue]
backend = "valkey"
url = "redis://valkey:6379"
```

Provides both `QueueProvider` (work distribution) and `DistributedLockProvider` (workflow-level locking).

## Search

Optional. When configured, enables:
- Full-text workflow log search via `SearchLogs` RPC
- Workflow instance indexing for filtered queries

```toml
[search]
url = "http://opensearch:9200"
```

## Authentication

### Static Bearer Tokens

Simplest auth. Tokens are compared in constant time.

```toml
[auth]
tokens = ["token1", "token2"]
```

Use with: `Authorization: Bearer token1`

### OIDC / JWT

For production. The server discovers JWKS keys from the OIDC issuer and validates JWT tokens on every request.

```toml
[auth]
oidc_issuer = "https://auth.sunbeam.pt/"
oidc_audience = "wfe-server"
```

Security properties:
- Algorithm derived from JWK (prevents algorithm confusion attacks)
- Symmetric algorithms rejected (RS256, RS384, RS512, ES256, ES384, PS256 only)
- OIDC issuer must use HTTPS
- Fail-closed: server won't start if OIDC discovery fails

Use with: `Authorization: Bearer <jwt-token>`

### Webhook HMAC

Webhook endpoints validate payloads using HMAC-SHA256.

```toml
[auth.webhook_secrets]
github = "your-github-webhook-secret"
gitea = "your-gitea-webhook-secret"
```

## gRPC API

13 RPCs available on the gRPC port (default 50051):

| RPC | Description |
|-----|-------------|
| `StartWorkflow` | Start a new workflow instance |
| `GetWorkflow` | Get workflow instance by ID |
| `ListWorkflows` | List workflow instances with filters |
| `SuspendWorkflow` | Pause a running workflow |
| `ResumeWorkflow` | Resume a suspended workflow |
| `TerminateWorkflow` | Stop a workflow permanently |
| `RegisterDefinition` | Register a workflow definition |
| `GetDefinition` | Get a workflow definition |
| `ListDefinitions` | List all registered definitions |
| `PublishEvent` | Publish an event for waiting workflows |
| `WatchLifecycle` | Server-streaming: lifecycle events |
| `StreamLogs` | Server-streaming: real-time step output |
| `SearchLogs` | Full-text search over step logs |

## HTTP Webhooks

Webhook endpoint: `POST /webhooks/{source}`

Supported sources:
- `github` -- GitHub webhook payloads with `X-Hub-Signature-256` HMAC
- `gitea` -- Gitea webhook payloads with `X-Gitea-Signature` HMAC
- `generic` -- Any JSON payload (requires bearer token auth)

Payload size limit: 2MB.

## Workflow YAML Auto-Loading

Point `workflows_dir` at a directory of `.yaml` files to auto-register workflow definitions on startup.

```toml
workflows_dir = "/etc/wfe/workflows"
```

File format: see [wfe-yaml](../wfe-yaml/) for the YAML workflow definition schema.

## Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 50051 | gRPC (HTTP/2) | Workflow API |
| 8080 | HTTP/1.1 | Webhooks |

## Health Check

The gRPC port responds to standard gRPC health checks. For HTTP health, any non-webhook GET to port 8080 returns 404 (the server is up if it responds).

## Environment Variable Reference

All configuration can be set via environment variables:

```bash
WFE_GRPC_ADDR=0.0.0.0:50051
WFE_HTTP_ADDR=0.0.0.0:8080
WFE_PERSISTENCE=postgres
WFE_DB_URL=postgres://wfe:secret@postgres:5432/wfe
WFE_QUEUE=valkey
WFE_QUEUE_URL=redis://valkey:6379
WFE_SEARCH_URL=http://opensearch:9200
WFE_WORKFLOWS_DIR=/etc/wfe/workflows
WFE_AUTH_TOKENS=token1,token2
RUST_LOG=info                    # Tracing filter (debug, info, warn, error)
```
