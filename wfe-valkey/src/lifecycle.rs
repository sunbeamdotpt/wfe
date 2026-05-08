use async_trait::async_trait;
use wfe_core::models::LifecycleEvent;
use wfe_core::traits::LifecyclePublisher;

/// Valkeylifecyclepublisher.
pub struct ValkeyLifecyclePublisher {
    conn: redis::aio::MultiplexedConnection,
    prefix: String,
}

impl ValkeyLifecyclePublisher {
    pub async fn new(redis_url: &str, prefix: &str) -> wfe_core::Result<Self> {
        let client = redis::Client::open(redis_url)
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;
        let conn = client
            .get_multiplexed_tokio_connection()
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;
        Ok(Self {
            conn,
            prefix: prefix.to_string(),
        })
    }
}

#[async_trait]
impl LifecyclePublisher for ValkeyLifecyclePublisher {
    async fn publish(&self, event: LifecycleEvent) -> wfe_core::Result<()> {
        let mut conn = self.conn.clone();
        let json = serde_json::to_string(&event)?;

        let instance_channel = format!("{}:lifecycle:{}", self.prefix, event.workflow_instance_id);
        let all_channel = format!("{}:lifecycle:all", self.prefix);

        // Publish to the instance-specific channel.
        redis::cmd("PUBLISH")
            .arg(&instance_channel)
            .arg(&json)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;

        // Publish to the global "all" channel.
        redis::cmd("PUBLISH")
            .arg(&all_channel)
            .arg(&json)
            .query_async::<i64>(&mut conn)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;

        Ok(())
    }
}
