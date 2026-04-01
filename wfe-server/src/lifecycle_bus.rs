use async_trait::async_trait;
use tokio::sync::broadcast;
use wfe_core::models::LifecycleEvent;
use wfe_core::traits::LifecyclePublisher;

/// Broadcasts lifecycle events to multiple subscribers via tokio broadcast channels.
pub struct BroadcastLifecyclePublisher {
    sender: broadcast::Sender<LifecycleEvent>,
}

impl BroadcastLifecyclePublisher {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LifecycleEvent> {
        self.sender.subscribe()
    }
}

#[async_trait]
impl LifecyclePublisher for BroadcastLifecyclePublisher {
    async fn publish(&self, event: LifecycleEvent) -> wfe_core::Result<()> {
        // Ignore send errors (no active subscribers).
        let _ = self.sender.send(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfe_core::models::LifecycleEventType;

    #[tokio::test]
    async fn publish_and_receive() {
        let bus = BroadcastLifecyclePublisher::new(16);
        let mut rx = bus.subscribe();

        let event = LifecycleEvent::new("wf-1", "def-1", 1, LifecycleEventType::Started);
        bus.publish(event.clone()).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.workflow_instance_id, "wf-1");
        assert_eq!(received.event_type, LifecycleEventType::Started);
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let bus = BroadcastLifecyclePublisher::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(LifecycleEvent::new("wf-1", "def-1", 1, LifecycleEventType::Completed))
            .await
            .unwrap();

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1.event_type, LifecycleEventType::Completed);
        assert_eq!(e2.event_type, LifecycleEventType::Completed);
    }

    #[tokio::test]
    async fn no_subscribers_does_not_error() {
        let bus = BroadcastLifecyclePublisher::new(16);
        // No subscribers — should not panic.
        bus.publish(LifecycleEvent::new("wf-1", "def-1", 1, LifecycleEventType::Started))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn step_events_propagate() {
        let bus = BroadcastLifecyclePublisher::new(16);
        let mut rx = bus.subscribe();

        bus.publish(LifecycleEvent::new(
            "wf-1",
            "def-1",
            1,
            LifecycleEventType::StepStarted {
                step_id: 3,
                step_name: Some("build".to_string()),
            },
        ))
        .await
        .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(
            received.event_type,
            LifecycleEventType::StepStarted {
                step_id: 3,
                step_name: Some("build".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn error_events_include_message() {
        let bus = BroadcastLifecyclePublisher::new(16);
        let mut rx = bus.subscribe();

        bus.publish(LifecycleEvent::new(
            "wf-1",
            "def-1",
            1,
            LifecycleEventType::Error {
                message: "step failed".to_string(),
            },
        ))
        .await
        .unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(
            received.event_type,
            LifecycleEventType::Error {
                message: "step failed".to_string(),
            }
        );
    }
}
