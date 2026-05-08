use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// A chunk of log output from a step execution.
#[derive(Debug, Clone)]
pub struct LogChunk {
    /// Workflow id.
    pub workflow_id: String,
    /// Definition id.
    pub definition_id: String,
    /// Step id.
    pub step_id: usize,
    /// Step name.
    pub step_name: String,
    /// Stream.
    pub stream: LogStreamType,
    /// Data.
    pub data: Vec<u8>,
    /// Timestamp.
    pub timestamp: DateTime<Utc>,
}

/// Whether a log chunk is from stdout or stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStreamType {
    /// Stdout.
    Stdout,
    /// Stderr.
    Stderr,
}

/// Receives log chunks as they're produced during step execution.
///
/// Implementations can broadcast to live subscribers, persist to a database,
/// index for search, or any combination. The trait is designed to be called
/// from within step executors (shell, containerd, etc.) as lines are produced.
#[async_trait]
pub trait LogSink: Send + Sync {
    async fn write_chunk(&self, chunk: LogChunk);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_stream_type_equality() {
        assert_eq!(LogStreamType::Stdout, LogStreamType::Stdout);
        assert_ne!(LogStreamType::Stdout, LogStreamType::Stderr);
    }

    #[test]
    fn log_chunk_clone() {
        let chunk = LogChunk {
            workflow_id: "wf-1".to_string(),
            definition_id: "def-1".to_string(),
            step_id: 0,
            step_name: "build".to_string(),
            stream: LogStreamType::Stdout,
            data: b"hello\n".to_vec(),
            timestamp: Utc::now(),
        };
        let cloned = chunk.clone();
        assert_eq!(cloned.workflow_id, "wf-1");
        assert_eq!(cloned.stream, LogStreamType::Stdout);
        assert_eq!(cloned.data, b"hello\n");
    }
}
