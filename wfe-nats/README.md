# wfe-nats

NATS/JetStream backend for the WFE workflow engine.

## Features

- **Queue provider** (`NatsQueueProvider`) — durable JetStream streams with push consumers and internal channels.
- **Lifecycle publisher** (`NatsLifecyclePublisher`) — NATS pub/sub with local broadcast bridging for clustered lifecycle events.
- **Distributed lock provider** (`NatsLockProvider`) — JetStream KV with TTL and heartbeats.
- **Client auth** — supports token, username/password, credentials file, and JWT/nkey (client side of NATS callout auth).

## Usage

```rust
use std::sync::Arc;
use wfe_nats::{NatsConfig, NatsQueueProvider, NatsLifecyclePublisher, NatsLockProvider};

#[tokio::main]
async fn main() -> wfe_core::Result<()> {
    let config = NatsConfig::default();
    let queue = Arc::new(NatsQueueProvider::new(config.clone()).await?);
    let lifecycle = Arc::new(NatsLifecyclePublisher::new(config.clone()).await?);
    let lock = Arc::new(NatsLockProvider::new(config).await?);
    Ok(())
}
```

## Testing

Integration tests require Docker and use the `sdk` testing feature:

```bash
cargo test -p wfe-nats --features testing
```

The `testing` feature pulls in `sdk` from its git remote and spins up a throwaway NATS container via `sdk::testing::Nats`.
