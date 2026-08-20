use std::sync::Arc;

use async_nats::jetstream::Context;

use crate::config::NatsConfig;

/// Shared NATS client and JetStream context.
#[derive(Debug, Clone)]
pub struct NatsClient {
    /// Core NATS client.
    pub client: async_nats::Client,
    /// JetStream context.
    pub jetstream: Context,
}

impl NatsClient {
    /// Connect to NATS and create a JetStream context.
    pub async fn connect(config: &NatsConfig) -> wfe_core::Result<Arc<Self>> {
        let client = crate::auth::connect(config).await?;
        let jetstream = async_nats::jetstream::new(client.clone());
        Ok(Arc::new(Self { client, jetstream }))
    }
}
