use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use wfe_core::models::LifecycleEvent;
use wfe_core::traits::LifecyclePublisher;

use crate::client::NatsClient;
use crate::config::NatsConfig;

/// Default broadcast channel capacity for local subscribers.
const BROADCAST_CAPACITY: usize = 4096;
/// NATS publish timeout.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);

/// NATS pub/sub-backed `LifecyclePublisher` with local broadcast bridging.
#[derive(Debug)]
pub struct NatsLifecyclePublisher {
    client: Arc<NatsClient>,
    config: NatsConfig,
    sender: broadcast::Sender<LifecycleEvent>,
    shutdown: CancellationToken,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl NatsLifecyclePublisher {
    /// Connect to NATS, prepare the publisher, and start the background subscriber.
    pub async fn new(config: NatsConfig) -> wfe_core::Result<Self> {
        let client = NatsClient::connect(&config).await?;
        let (sender, _) = broadcast::channel(BROADCAST_CAPACITY);
        let publisher = Self {
            client,
            config,
            sender,
            shutdown: CancellationToken::new(),
            handle: Mutex::new(None),
        };
        publisher.start_subscriber().await?;
        Ok(publisher)
    }

    fn instance_subject(prefix: &str, instance_id: &str) -> String {
        format!("{}.lifecycle.{}", prefix, instance_id)
    }

    fn all_subject(prefix: &str) -> String {
        format!("{}.lifecycle.all", prefix)
    }

    fn wildcard_subject(prefix: &str) -> String {
        format!("{}.lifecycle.>", prefix)
    }

    async fn start_subscriber(&self) -> wfe_core::Result<()> {
        let subject = Self::wildcard_subject(&self.config.prefix);
        let mut subscriber = self
            .client
            .client
            .subscribe(subject)
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(format!("failed to subscribe: {e}")))?;

        let sender = self.sender.clone();
        let shutdown = self.shutdown.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        debug!("lifecycle subscriber shutting down");
                        break;
                    }
                    message = subscriber.next() => {
                        match message {
                            Some(message) => {
                                let event: LifecycleEvent = match serde_json::from_slice(&message.payload) {
                                    Ok(event) => event,
                                    Err(e) => {
                                        error!(error = %e, payload = %String::from_utf8_lossy(&message.payload), "failed to deserialize lifecycle event");
                                        continue;
                                    }
                                };
                                let _ = sender.send(event);
                            }
                            None => {
                                warn!("lifecycle subscriber stream ended");
                                break;
                            }
                        }
                    }
                }
            }
        });

        *self.handle.lock().await = Some(handle);
        Ok(())
    }

    /// Start the background NATS subscriber that bridges remote events into local subscribers.
    ///
    /// This is called automatically by `new`; exposed separately for testing.
    pub async fn start(&self) -> wfe_core::Result<()> {
        self.start_subscriber().await
    }

    /// Stop the background subscriber.
    pub async fn stop(&self) -> wfe_core::Result<()> {
        self.shutdown.cancel();
        if let Some(handle) = self.handle.lock().await.take()
            && let Err(e) = handle.await
        {
            warn!(error = %e, "lifecycle subscriber task panicked");
        }
        Ok(())
    }
}

#[async_trait]
impl LifecyclePublisher for NatsLifecyclePublisher {
    async fn publish(&self, event: LifecycleEvent) -> wfe_core::Result<()> {
        let payload = serde_json::to_vec(&event)?;
        let instance_subject = Self::instance_subject(&self.config.prefix, &event.workflow_instance_id);
        let all_subject = Self::all_subject(&self.config.prefix);

        for subject in [instance_subject, all_subject] {
            let publish = self
                .client
                .client
                .publish(subject, payload.clone().into());

            match tokio::time::timeout(PUBLISH_TIMEOUT, publish).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(wfe_core::WfeError::Persistence(format!(
                        "failed to publish lifecycle event: {e}"
                    )));
                }
                Err(_) => {
                    return Err(wfe_core::WfeError::Persistence(
                        "lifecycle event publish timed out".into(),
                    ));
                }
            }
        }

        // Also broadcast locally so subscribers on this instance see the event immediately.
        let _ = self.sender.send(event);
        Ok(())
    }

    fn subscribe(&self) -> wfe_core::Result<broadcast::Receiver<LifecycleEvent>> {
        Ok(self.sender.subscribe())
    }
}
