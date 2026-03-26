# wfe-valkey

Valkey (Redis-compatible) provider for distributed locking, queues, and lifecycle events in WFE.

## What it does

Provides three provider implementations backed by Valkey (or any Redis-compatible server) using the `redis` crate with multiplexed async connections. Handles distributed lock coordination across multiple WFE host instances, work queue distribution for workflows/events/indexing, and pub/sub lifecycle event broadcasting.

## Quick start

```rust
use wfe_valkey::{ValkeyLockProvider, ValkeyQueueProvider, ValkeyLifecyclePublisher};

let redis_url = "redis://127.0.0.1:6379";
let prefix = "wfe"; // key prefix for namespacing

let locks = ValkeyLockProvider::new(redis_url, prefix).await?;
let queues = ValkeyQueueProvider::new(redis_url, prefix).await?;
let lifecycle = ValkeyLifecyclePublisher::new(redis_url, prefix).await?;
```

Custom lock duration (default is 30 seconds):

```rust
use std::time::Duration;

let locks = ValkeyLockProvider::new(redis_url, prefix).await?
    .with_lock_duration(Duration::from_secs(10));
```

## API

| Type | Trait | Purpose |
|------|-------|---------|
| `ValkeyLockProvider` | `DistributedLockProvider` | Distributed mutex via `SET NX EX`. Release uses a Lua script to ensure only the holder can unlock. |
| `ValkeyQueueProvider` | `QueueProvider` | FIFO work queues via `LPUSH`/`RPOP`. Separate queues for `Workflow`, `Event`, and `Index` work types. |
| `ValkeyLifecyclePublisher` | `LifecyclePublisher` | Pub/sub lifecycle events. Publishes to both an instance-specific channel and a global `all` channel. |

### Key layout

All keys are prefixed with the configured prefix (e.g., `wfe`):

| Pattern | Usage |
|---------|-------|
| `{prefix}:lock:{resource}` | Distributed lock for a resource |
| `{prefix}:queue:workflow` | Workflow processing queue |
| `{prefix}:queue:event` | Event processing queue |
| `{prefix}:queue:index` | Index update queue |
| `{prefix}:lifecycle:{workflow_id}` | Lifecycle events for a specific workflow |
| `{prefix}:lifecycle:all` | All lifecycle events |

## Configuration

Connection string is a standard Redis URI:

```
redis://127.0.0.1:6379
redis://:password@host:6379/0
```

Each provider creates its own multiplexed connection. The prefix parameter namespaces all keys, so multiple WFE deployments can share a single Valkey instance.

## Testing

Requires a running Valkey instance. Use the project docker-compose:

```sh
docker compose up -d valkey
cargo test -p wfe-valkey
```

Default test connection: `redis://127.0.0.1:6379`

## License

MIT
