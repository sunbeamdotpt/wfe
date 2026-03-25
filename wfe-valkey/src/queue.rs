use async_trait::async_trait;
use redis::AsyncCommands;
use wfe_core::models::QueueType;
use wfe_core::traits::QueueProvider;

pub struct ValkeyQueueProvider {
    conn: redis::aio::MultiplexedConnection,
    prefix: String,
}

impl ValkeyQueueProvider {
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

    fn queue_key(&self, queue_type: QueueType) -> String {
        let type_str = match queue_type {
            QueueType::Workflow => "workflow",
            QueueType::Event => "event",
            QueueType::Index => "index",
        };
        format!("{}:queue:{}", self.prefix, type_str)
    }
}

#[async_trait]
impl QueueProvider for ValkeyQueueProvider {
    async fn queue_work(&self, id: &str, queue: QueueType) -> wfe_core::Result<()> {
        let mut conn = self.conn.clone();
        let key = self.queue_key(queue);

        conn.lpush::<_, _, ()>(&key, id)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;

        Ok(())
    }

    async fn dequeue_work(&self, queue: QueueType) -> wfe_core::Result<Option<String>> {
        let mut conn = self.conn.clone();
        let key = self.queue_key(queue);

        let result: Option<String> = conn
            .rpop(&key, None)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;

        Ok(result)
    }

    fn is_dequeue_blocking(&self) -> bool {
        false
    }

    async fn start(&self) -> wfe_core::Result<()> {
        // No-op.
        Ok(())
    }

    async fn stop(&self) -> wfe_core::Result<()> {
        // No-op.
        Ok(())
    }
}
