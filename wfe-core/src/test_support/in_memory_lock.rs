use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::traits::DistributedLockProvider;
use crate::Result;

/// An in-memory implementation of `DistributedLockProvider` for testing.
#[derive(Debug, Clone)]
pub struct InMemoryLockProvider {
    locks: Arc<Mutex<HashSet<String>>>,
}

impl InMemoryLockProvider {
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl Default for InMemoryLockProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DistributedLockProvider for InMemoryLockProvider {
    async fn acquire_lock(&self, resource: &str) -> Result<bool> {
        let mut locks = self.locks.lock().await;
        Ok(locks.insert(resource.to_string()))
    }

    async fn release_lock(&self, resource: &str) -> Result<()> {
        let mut locks = self.locks.lock().await;
        locks.remove(resource);
        Ok(())
    }

    async fn start(&self) -> Result<()> {
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}
