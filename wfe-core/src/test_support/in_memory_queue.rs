use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::models::QueueType;
use crate::traits::QueueProvider;
use crate::Result;

/// An in-memory implementation of `QueueProvider` for testing.
#[derive(Debug, Clone)]
pub struct InMemoryQueueProvider {
    queues: Arc<Mutex<HashMap<QueueType, VecDeque<String>>>>,
}

impl InMemoryQueueProvider {
    pub fn new() -> Self {
        Self {
            queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryQueueProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl QueueProvider for InMemoryQueueProvider {
    async fn queue_work(&self, id: &str, queue: QueueType) -> Result<()> {
        let mut queues = self.queues.lock().await;
        queues
            .entry(queue)
            .or_insert_with(VecDeque::new)
            .push_back(id.to_string());
        Ok(())
    }

    async fn dequeue_work(&self, queue: QueueType) -> Result<Option<String>> {
        let mut queues = self.queues.lock().await;
        Ok(queues.get_mut(&queue).and_then(|q| q.pop_front()))
    }

    fn is_dequeue_blocking(&self) -> bool {
        false
    }

    async fn start(&self) -> Result<()> {
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}
