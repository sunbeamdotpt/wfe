use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::broadcast;
use wfe_core::traits::log_sink::{LogChunk, LogSink};

/// Stores and broadcasts log chunks for workflow step executions.
///
/// Three tiers:
/// 1. **Live broadcast** — per-workflow broadcast channel for StreamLogs subscribers
/// 2. **In-memory history** — append-only buffer per (workflow_id, step_id) for replay
/// 3. **Search index** — OpenSearch log indexing via LogSearchIndex (optional)
pub struct LogStore {
    /// Per-workflow broadcast channels for live streaming.
    live: DashMap<String, broadcast::Sender<LogChunk>>,
    /// In-memory history per (workflow_id, step_id).
    history: DashMap<(String, usize), Vec<LogChunk>>,
    /// Optional search index for log lines.
    search: Option<Arc<crate::log_search::LogSearchIndex>>,
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            live: DashMap::new(),
            history: DashMap::new(),
            search: None,
        }
    }

    pub fn with_search(mut self, index: Arc<crate::log_search::LogSearchIndex>) -> Self {
        self.search = Some(index);
        self
    }

    /// Subscribe to live log chunks for a workflow.
    pub fn subscribe(&self, workflow_id: &str) -> broadcast::Receiver<LogChunk> {
        self.live
            .entry(workflow_id.to_string())
            .or_insert_with(|| broadcast::channel(4096).0)
            .subscribe()
    }

    /// Get historical logs for a workflow, optionally filtered by step.
    pub fn get_history(&self, workflow_id: &str, step_id: Option<usize>) -> Vec<LogChunk> {
        let mut result = Vec::new();
        for entry in self.history.iter() {
            let (wf_id, s_id) = entry.key();
            if wf_id != workflow_id {
                continue;
            }
            if let Some(filter_step) = step_id {
                if *s_id != filter_step {
                    continue;
                }
            }
            result.extend(entry.value().iter().cloned());
        }
        // Sort by timestamp.
        result.sort_by_key(|c| c.timestamp);
        result
    }
}

#[async_trait]
impl LogSink for LogStore {
    async fn write_chunk(&self, chunk: LogChunk) {
        // Store in history.
        self.history
            .entry((chunk.workflow_id.clone(), chunk.step_id))
            .or_default()
            .push(chunk.clone());

        // Broadcast to live subscribers.
        let sender = self
            .live
            .entry(chunk.workflow_id.clone())
            .or_insert_with(|| broadcast::channel(4096).0);
        let _ = sender.send(chunk.clone());

        // Index to OpenSearch (best-effort, don't block on failure).
        if let Some(ref search) = self.search {
            if let Err(e) = search.index_chunk(&chunk).await {
                tracing::warn!(error = %e, "failed to index log chunk to OpenSearch");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use wfe_core::traits::LogStreamType;

    fn make_chunk(workflow_id: &str, step_id: usize, step_name: &str, data: &str) -> LogChunk {
        LogChunk {
            workflow_id: workflow_id.to_string(),
            definition_id: "def-1".to_string(),
            step_id,
            step_name: step_name.to_string(),
            stream: LogStreamType::Stdout,
            data: data.as_bytes().to_vec(),
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn write_and_read_history() {
        let store = LogStore::new();
        store.write_chunk(make_chunk("wf-1", 0, "build", "line 1\n")).await;
        store.write_chunk(make_chunk("wf-1", 0, "build", "line 2\n")).await;

        let history = store.get_history("wf-1", None);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].data, b"line 1\n");
        assert_eq!(history[1].data, b"line 2\n");
    }

    #[tokio::test]
    async fn history_filtered_by_step() {
        let store = LogStore::new();
        store.write_chunk(make_chunk("wf-1", 0, "build", "build log\n")).await;
        store.write_chunk(make_chunk("wf-1", 1, "test", "test log\n")).await;

        let build_only = store.get_history("wf-1", Some(0));
        assert_eq!(build_only.len(), 1);
        assert_eq!(build_only[0].step_name, "build");

        let test_only = store.get_history("wf-1", Some(1));
        assert_eq!(test_only.len(), 1);
        assert_eq!(test_only[0].step_name, "test");
    }

    #[tokio::test]
    async fn empty_history_for_unknown_workflow() {
        let store = LogStore::new();
        assert!(store.get_history("nonexistent", None).is_empty());
    }

    #[tokio::test]
    async fn live_broadcast() {
        let store = LogStore::new();
        let mut rx = store.subscribe("wf-1");

        store.write_chunk(make_chunk("wf-1", 0, "build", "hello\n")).await;

        let received = rx.recv().await.unwrap();
        assert_eq!(received.data, b"hello\n");
        assert_eq!(received.workflow_id, "wf-1");
    }

    #[tokio::test]
    async fn broadcast_different_workflows_isolated() {
        let store = LogStore::new();
        let mut rx1 = store.subscribe("wf-1");
        let mut rx2 = store.subscribe("wf-2");

        store.write_chunk(make_chunk("wf-1", 0, "build", "wf1 log\n")).await;
        store.write_chunk(make_chunk("wf-2", 0, "test", "wf2 log\n")).await;

        let e1 = rx1.recv().await.unwrap();
        assert_eq!(e1.workflow_id, "wf-1");

        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e2.workflow_id, "wf-2");
    }

    #[tokio::test]
    async fn no_subscribers_does_not_error() {
        let store = LogStore::new();
        // No subscribers — should not panic.
        store.write_chunk(make_chunk("wf-1", 0, "build", "orphan log\n")).await;
        // History should still be stored.
        assert_eq!(store.get_history("wf-1", None).len(), 1);
    }

    #[tokio::test]
    async fn multiple_subscribers_same_workflow() {
        let store = LogStore::new();
        let mut rx1 = store.subscribe("wf-1");
        let mut rx2 = store.subscribe("wf-1");

        store.write_chunk(make_chunk("wf-1", 0, "build", "shared\n")).await;

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();
        assert_eq!(e1.data, b"shared\n");
        assert_eq!(e2.data, b"shared\n");
    }

    #[tokio::test]
    async fn history_preserves_stream_type() {
        let store = LogStore::new();
        let mut chunk = make_chunk("wf-1", 0, "build", "error output\n");
        chunk.stream = LogStreamType::Stderr;
        store.write_chunk(chunk).await;

        let history = store.get_history("wf-1", None);
        assert_eq!(history[0].stream, LogStreamType::Stderr);
    }
}
