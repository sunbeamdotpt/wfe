use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use wfe_core::traits::DistributedLockProvider;

use crate::client::NatsClient;
use crate::config::NatsConfig;

/// JetStream KV-backed `DistributedLockProvider`.
///
/// # Limitations
///
/// JetStream KV is not a true distributed lock. This implementation relies on
/// `create` semantics, a short TTL, and a heartbeat task to keep locks alive.
/// Clock skew between the NATS server and clients can affect correctness, and a
/// heartbeat may overwrite a lock acquired by another instance after an expiry.
/// Use this backend only where the existing WFE executor idempotency guarantees
/// make that acceptable.
#[derive(Debug)]
pub struct NatsLockProvider {
    config: NatsConfig,
    instance_id: String,
    kv: async_nats::jetstream::kv::Store,
    held_locks: Arc<Mutex<HashSet<String>>>,
    shutdown: CancellationToken,
    heartbeat_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl NatsLockProvider {
    /// Connect to NATS and create the KV bucket.
    pub async fn new(config: NatsConfig) -> wfe_core::Result<Self> {
        let client = NatsClient::connect(&config).await?;
        let instance_id = uuid::Uuid::new_v4().to_string();
        let kv = Self::create_kv(&client, &config).await?;

        Ok(Self {
            config,
            instance_id,
            kv,
            held_locks: Arc::new(Mutex::new(HashSet::new())),
            shutdown: CancellationToken::new(),
            heartbeat_handle: Mutex::new(None),
        })
    }

    async fn create_kv(
        client: &Arc<NatsClient>,
        config: &NatsConfig,
    ) -> wfe_core::Result<async_nats::jetstream::kv::Store> {
        let max_age = Duration::from_secs(config.lock_ttl_secs);
        client
            .jetstream
            .create_key_value(async_nats::jetstream::kv::Config {
                bucket: config.kv_bucket.clone(),
                history: 1,
                max_age,
                ..Default::default()
            })
            .await
            .map_err(|e| {
                wfe_core::WfeError::Persistence(format!(
                    "failed to create kv bucket {}: {e}",
                    config.kv_bucket
                ))
            })
    }

    fn lock_key(prefix: &str, resource: &str) -> String {
        format!("{}.lock.{}", prefix, resource)
    }

    async fn start_heartbeat(&self) {
        let kv = self.kv.clone();
        let held_locks = Arc::clone(&self.held_locks);
        let instance_id = self.instance_id.clone();
        let shutdown = self.shutdown.clone();
        let ttl = Duration::from_secs(self.config.lock_ttl_secs);
        let interval = ttl / 3;
        let prefix = self.config.prefix.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        debug!("lock heartbeat shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(interval) => {
                        let locks: Vec<String> = {
                            let guard = held_locks.lock().await;
                            guard.iter().cloned().collect()
                        };

                        for resource in locks {
                            let key = Self::lock_key(&prefix, &resource);
                            if let Err(e) = kv.put(&key, instance_id.clone().into()).await {
                                error!(resource = %resource, error = %e, "failed to refresh lock");
                            }
                        }
                    }
                }
            }
        });

        *self.heartbeat_handle.lock().await = Some(handle);
    }
}

#[async_trait]
impl DistributedLockProvider for NatsLockProvider {
    async fn acquire_lock(&self, resource: &str) -> wfe_core::Result<bool> {
        let key = Self::lock_key(&self.config.prefix, resource);
        let value = bytes::Bytes::from(self.instance_id.clone());

        match self.kv.create(&key, value).await {
            Ok(_) => {
                self.held_locks.lock().await.insert(resource.to_string());
                debug!(resource = %resource, "acquired nats kv lock");
                Ok(true)
            }
            Err(e)
                if e.kind() == async_nats::jetstream::kv::CreateErrorKind::AlreadyExists =>
            {
                Ok(false)
            }
            Err(e) => Err(wfe_core::WfeError::LockFailed(format!(
                "failed to acquire lock for {resource}: {e}"
            ))),
        }
    }

    async fn release_lock(&self, resource: &str) -> wfe_core::Result<()> {
        let key = Self::lock_key(&self.config.prefix, resource);

        // Only delete if we still hold the lock to avoid releasing someone else's lock.
        match self.kv.entry(&key).await {
            Ok(Some(entry)) => {
                let holder = String::from_utf8_lossy(&entry.value);
                if holder == self.instance_id {
                    if let Err(e) = self.kv.delete(&key).await {
                        error!(resource = %resource, error = %e, "failed to delete lock");
                    }
                } else {
                    warn!(resource = %resource, holder = %holder, "lock is held by another instance");
                }
            }
            Ok(None) => {
                debug!(resource = %resource, "lock already released");
            }
            Err(e) => {
                warn!(resource = %resource, error = %e, "failed to read lock entry during release");
            }
        }

        self.held_locks.lock().await.remove(resource);
        Ok(())
    }

    async fn start(&self) -> wfe_core::Result<()> {
        self.start_heartbeat().await;
        Ok(())
    }

    async fn stop(&self) -> wfe_core::Result<()> {
        self.shutdown.cancel();
        if let Some(handle) = self.heartbeat_handle.lock().await.take()
            && let Err(e) = handle.await
        {
            warn!(error = %e, "heartbeat task panicked");
        }
        Ok(())
    }
}
