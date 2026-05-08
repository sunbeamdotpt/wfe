use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;
use wfe_core::traits::DistributedLockProvider;

/// Valkeylockprovider.
pub struct ValkeyLockProvider {
    conn: redis::aio::MultiplexedConnection,
    prefix: String,
    lock_duration: Duration,
    /// Unique identifier for this provider instance, used as the lock value.
    instance_id: String,
}

impl ValkeyLockProvider {
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
            lock_duration: Duration::from_secs(30),
            instance_id: Uuid::new_v4().to_string(),
        })
    }

    /// Create a provider with a custom lock duration.
    pub fn with_lock_duration(mut self, duration: Duration) -> Self {
        self.lock_duration = duration;
        self
    }

    fn lock_key(&self, resource: &str) -> String {
        format!("{}:lock:{}", self.prefix, resource)
    }
}

#[async_trait]
impl DistributedLockProvider for ValkeyLockProvider {
    async fn acquire_lock(&self, resource: &str) -> wfe_core::Result<bool> {
        let mut conn = self.conn.clone();
        let key = self.lock_key(resource);
        let seconds = self.lock_duration.as_secs();

        // SET key value NX EX seconds
        let result: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg(&self.instance_id)
            .arg("NX")
            .arg("EX")
            .arg(seconds)
            .query_async(&mut conn)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;

        Ok(result.is_some())
    }

    async fn release_lock(&self, resource: &str) -> wfe_core::Result<()> {
        let mut conn = self.conn.clone();
        let key = self.lock_key(resource);

        // Use a Lua script to only delete the key if the value matches our instance_id.
        // This prevents accidentally releasing a lock held by another instance.
        let script = redis::Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("DEL", KEYS[1])
            else
                return 0
            end
            "#,
        );

        let _: i64 = script
            .key(&key)
            .arg(&self.instance_id)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(e.to_string()))?;

        Ok(())
    }

    async fn start(&self) -> wfe_core::Result<()> {
        // No-op: Redis/Valkey is always-on.
        Ok(())
    }

    async fn stop(&self) -> wfe_core::Result<()> {
        // No-op.
        Ok(())
    }
}
