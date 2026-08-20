use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

use wfe_core::models::QueueType;
use wfe_core::traits::QueueProvider;

use crate::client::NatsClient;
use crate::config::NatsConfig;

/// Capacity of the internal channel between the push consumer and `dequeue_work`.
const CHANNEL_CAPACITY: usize = 256;
/// Timeout used when receiving from the internal channel in `dequeue_work`.
const DEQUEUE_TIMEOUT: Duration = Duration::from_millis(100);

/// JetStream-backed `QueueProvider` using durable push consumers.
#[derive(Debug)]
pub struct NatsQueueProvider {
    client: Arc<NatsClient>,
    #[allow(dead_code)]
    config: NatsConfig,
    instance_id: String,
    channels: Mutex<HashMap<QueueType, mpsc::Receiver<String>>>,
    senders: HashMap<QueueType, mpsc::Sender<String>>,
    shutdown: CancellationToken,
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl NatsQueueProvider {
    /// Connect to NATS, create streams/consumers, and prepare internal channels.
    pub async fn new(config: NatsConfig) -> wfe_core::Result<Self> {
        let client = NatsClient::connect(&config).await?;
        let instance_id = uuid::Uuid::new_v4().to_string();

        for queue_type in [QueueType::Workflow, QueueType::Event, QueueType::Index] {
            Self::ensure_stream_and_consumer(&client, &config, queue_type, &instance_id).await?;
        }

        let mut channels = HashMap::new();
        let mut senders = HashMap::new();
        for queue_type in [QueueType::Workflow, QueueType::Event, QueueType::Index] {
            let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
            channels.insert(queue_type, rx);
            senders.insert(queue_type, tx);
        }

        Ok(Self {
            client,
            config,
            instance_id,
            channels: Mutex::new(channels),
            senders,
            shutdown: CancellationToken::new(),
            handles: Mutex::new(Vec::new()),
        })
    }

    fn queue_type_str(queue_type: QueueType) -> &'static str {
        match queue_type {
            QueueType::Workflow => "workflow",
            QueueType::Event => "event",
            QueueType::Index => "index",
        }
    }

    fn subject(queue_type: QueueType, prefix: &str) -> String {
        format!("{}.queue.{}", prefix, Self::queue_type_str(queue_type))
    }

    fn stream_name(queue_type: QueueType, base: &str) -> String {
        format!("{}_{}", base, Self::queue_type_str(queue_type))
    }

    fn consumer_name(queue_type: QueueType, base: &str, instance_id: &str) -> String {
        format!(
            "{}_{}_{}",
            base,
            Self::queue_type_str(queue_type),
            instance_id
        )
    }

    fn deliver_subject(queue_type: QueueType, prefix: &str, instance_id: &str) -> String {
        format!(
            "{}.delivery.{}.{}",
            prefix,
            Self::queue_type_str(queue_type),
            instance_id
        )
    }

    async fn ensure_stream_and_consumer(
        client: &Arc<NatsClient>,
        config: &NatsConfig,
        queue_type: QueueType,
        instance_id: &str,
    ) -> wfe_core::Result<()> {
        let stream_name = Self::stream_name(queue_type, &config.stream_name);
        let subject = Self::subject(queue_type, &config.prefix);

        debug!(stream = %stream_name, subject = %subject, "creating jetstream stream");
        client
            .jetstream
            .get_or_create_stream(async_nats::jetstream::stream::Config {
                name: stream_name.clone(),
                subjects: vec![subject],
                ..Default::default()
            })
            .await
            .map_err(|e| {
                wfe_core::WfeError::Persistence(format!(
                    "failed to create stream {stream_name}: {e}"
                ))
            })?;

        let consumer_name = Self::consumer_name(queue_type, &config.consumer_name, instance_id);
        let deliver_subject = Self::deliver_subject(queue_type, &config.prefix, instance_id);
        let ack_wait = Duration::from_secs(config.ack_wait_secs);

        debug!(consumer = %consumer_name, deliver = %deliver_subject, "creating push consumer");
        let stream = client
            .jetstream
            .get_stream(&stream_name)
            .await
            .map_err(|e| {
                wfe_core::WfeError::Persistence(format!(
                    "failed to get stream {stream_name}: {e}"
                ))
            })?;

        stream
            .get_or_create_consumer(
                &consumer_name,
                async_nats::jetstream::consumer::push::Config {
                    durable_name: Some(consumer_name.clone()),
                    deliver_subject,
                    ack_wait,
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| {
                wfe_core::WfeError::Persistence(format!(
                    "failed to create consumer {consumer_name}: {e}"
                ))
            })?;

        Ok(())
    }

    async fn consumer_for_queue(
        &self,
        queue_type: QueueType,
    ) -> wfe_core::Result<async_nats::jetstream::consumer::PushConsumer> {
        let stream_name = Self::stream_name(queue_type, &self.config.stream_name);
        let consumer_name =
            Self::consumer_name(queue_type, &self.config.consumer_name, &self.instance_id);

        let stream = self
            .client
            .jetstream
            .get_stream(&stream_name)
            .await
            .map_err(|e| {
                wfe_core::WfeError::Persistence(format!(
                    "failed to get stream {stream_name}: {e}"
                ))
            })?;

        stream
            .get_consumer(&consumer_name)
            .await
            .map_err(|e| {
                wfe_core::WfeError::Persistence(format!(
                    "failed to get consumer {consumer_name}: {e}"
                ))
            })
    }

    async fn spawn_consumer_task(
        &self,
        queue_type: QueueType,
        sender: mpsc::Sender<String>,
    ) -> wfe_core::Result<()> {
        let consumer = self.consumer_for_queue(queue_type).await?;
        let mut messages = consumer.messages().await.map_err(|e| {
            wfe_core::WfeError::Persistence(format!(
                "failed to start message stream for {queue_type:?}: {e}"
            ))
        })?;

        let shutdown = self.shutdown.clone();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        debug!(queue = ?queue_type, "queue consumer shutting down");
                        break;
                    }
                    message = messages.next() => {
                        match message {
                            Some(Ok(message)) => {
                                let payload = match std::str::from_utf8(&message.payload) {
                                    Ok(s) => s.to_string(),
                                    Err(e) => {
                                        error!(queue = ?queue_type, error = %e, "invalid utf8 in queue message");
                                        continue;
                                    }
                                };

                                if let Err(e) = sender.try_send(payload) {
                                    warn!(queue = ?queue_type, error = %e, "internal queue full; message will be redelivered");
                                    continue;
                                }

                                if let Err(e) = message.ack().await {
                                    error!(queue = ?queue_type, error = %e, "failed to ack message");
                                }
                            }
                            Some(Err(e)) => {
                                error!(queue = ?queue_type, error = %e, "error from subscriber");
                            }
                            None => {
                                warn!(queue = ?queue_type, "subscriber stream ended");
                                break;
                            }
                        }
                    }
                }
            }
        });

        self.handles.lock().await.push(handle);
        Ok(())
    }
}

#[async_trait]
impl QueueProvider for NatsQueueProvider {
    async fn queue_work(&self, id: &str, queue: QueueType) -> wfe_core::Result<()> {
        let subject = Self::subject(queue, &self.config.prefix);
        self.client
            .jetstream
            .publish(subject, bytes::Bytes::from(id.to_string()))
            .await
            .map_err(|e| wfe_core::WfeError::Persistence(format!("failed to publish: {e}")))?;
        Ok(())
    }

    async fn dequeue_work(&self, queue: QueueType) -> wfe_core::Result<Option<String>> {
        let mut channels = self.channels.lock().await;
        let receiver = channels
            .get_mut(&queue)
            .ok_or_else(|| wfe_core::WfeError::Persistence("unknown queue type".into()))?;

        match tokio::time::timeout(DEQUEUE_TIMEOUT, receiver.recv()).await {
            Ok(Some(id)) => Ok(Some(id)),
            Ok(None) => Ok(None),
            Err(_) => Ok(None),
        }
    }

    fn is_dequeue_blocking(&self) -> bool {
        true
    }

    async fn start(&self) -> wfe_core::Result<()> {
        for queue_type in [QueueType::Workflow, QueueType::Event, QueueType::Index] {
            let sender = self.senders.get(&queue_type).cloned().ok_or_else(|| {
                wfe_core::WfeError::Persistence(format!("missing sender for queue {queue_type:?}"))
            })?;
            self.spawn_consumer_task(queue_type, sender).await?;
        }
        Ok(())
    }

    async fn stop(&self) -> wfe_core::Result<()> {
        self.shutdown.cancel();
        let mut handles = self.handles.lock().await;
        for handle in handles.drain(..) {
            if let Err(e) = handle.await {
                warn!(error = %e, "queue consumer task panicked");
            }
        }
        Ok(())
    }
}
