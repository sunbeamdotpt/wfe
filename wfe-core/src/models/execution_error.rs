use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Executionerror.
pub struct ExecutionError {
    /// Error time.
    pub error_time: DateTime<Utc>,
    /// Workflow id.
    pub workflow_id: String,
    /// Execution pointer id.
    pub execution_pointer_id: String,
    /// Message.
    pub message: String,
}

impl ExecutionError {
    pub fn new(
        workflow_id: impl Into<String>,
        execution_pointer_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            error_time: Utc::now(),
            workflow_id: workflow_id.into(),
            execution_pointer_id: execution_pointer_id.into(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn new_error_captures_fields() {
        let err = ExecutionError::new("wf-1", "ptr-1", "something went wrong");
        assert_eq!(err.workflow_id, "wf-1");
        assert_eq!(err.execution_pointer_id, "ptr-1");
        assert_eq!(err.message, "something went wrong");
    }

    #[test]
    fn serde_round_trip() {
        let err = ExecutionError::new("wf-1", "ptr-1", "fail");
        let json = serde_json::to_string(&err).unwrap();
        let deserialized: ExecutionError = serde_json::from_str(&json).unwrap();
        assert_eq!(err.workflow_id, deserialized.workflow_id);
        assert_eq!(err.message, deserialized.message);
    }
}
