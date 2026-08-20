use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, broadcast};

use crate::Result;
use crate::models::LifecycleEvent;
use crate::traits::LifecyclePublisher;

/// Default broadcast channel capacity for lifecycle events.
const DEFAULT_CAPACITY: usize = 256;

/// An in-memory implementation of `LifecyclePublisher` for testing.
#[derive(Debug, Clone)]
pub struct InMemoryLifecyclePublisher {
    events: Arc<Mutex<Vec<LifecycleEvent>>>,
    sender: broadcast::Sender<LifecycleEvent>,
}

impl InMemoryLifecyclePublisher {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_CAPACITY);
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            sender,
        }
    }

    /// Retrieve all published lifecycle events for assertions.
    pub async fn events(&self) -> Vec<LifecycleEvent> {
        self.events.lock().await.clone()
    }
}

impl Default for InMemoryLifecyclePublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LifecyclePublisher for InMemoryLifecyclePublisher {
    async fn publish(&self, event: LifecycleEvent) -> Result<()> {
        self.events.lock().await.push(event.clone());
        let _ = self.sender.send(event);
        Ok(())
    }

    fn subscribe(&self) -> Result<broadcast::Receiver<LifecycleEvent>> {
        Ok(self.sender.subscribe())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::LifecycleEventType;
    use crate::traits::LifecyclePublisher;

    #[test]
    fn default_impl() {
        let lc = InMemoryLifecyclePublisher::default();
        drop(lc);
    }

    #[tokio::test]
    async fn publish_and_retrieve_events() {
        let lc = InMemoryLifecyclePublisher::new();
        let event = LifecycleEvent::new("wf-1", "def-1", 1, LifecycleEventType::Started);
        lc.publish(event).await.unwrap();

        let events = lc.events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, LifecycleEventType::Started);
    }
}
