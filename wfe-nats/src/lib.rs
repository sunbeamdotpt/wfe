//! NATS/JetStream backend for the WFE workflow engine.
//!
//! Provides queue, lifecycle event pub/sub, and distributed lock implementations
//! backed by NATS JetStream and core NATS pub/sub.
//!
//! # Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use wfe_nats::{NatsConfig, NatsQueueProvider, NatsLifecyclePublisher, NatsLockProvider};
//!
//! # #[tokio::main]
//! # async fn main() -> wfe_core::Result<()> {
//! let config = NatsConfig::default();
//! let queue = Arc::new(NatsQueueProvider::new(config.clone()).await?);
//! let lifecycle = Arc::new(NatsLifecyclePublisher::new(config.clone()).await?);
//! let lock = Arc::new(NatsLockProvider::new(config).await?);
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod client;
pub mod config;
pub mod lifecycle;
pub mod lock;
pub mod queue;

pub use config::{NatsAuthConfig, NatsConfig};
pub use lifecycle::NatsLifecyclePublisher;
pub use lock::NatsLockProvider;
pub use queue::NatsQueueProvider;
