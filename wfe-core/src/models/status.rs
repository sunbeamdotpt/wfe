use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Workflowstatus.
pub enum WorkflowStatus {
    #[default]
    /// Runnable.
    Runnable,
    /// Suspended.
    Suspended,
    /// Complete.
    Complete,
    /// Terminated.
    Terminated,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Pointerstatus.
pub enum PointerStatus {
    #[default]
    /// Pending.
    Pending,
    /// Running.
    Running,
    /// Complete.
    Complete,
    /// Skipped.
    Skipped,
    /// Sleeping.
    Sleeping,
    /// Waitingforevent.
    WaitingForEvent,
    /// Failed.
    Failed,
    /// Compensated.
    Compensated,
    /// Cancelled.
    Cancelled,
    /// Pendingpredecessor.
    PendingPredecessor,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn workflow_status_default_is_runnable() {
        assert_eq!(WorkflowStatus::default(), WorkflowStatus::Runnable);
    }

    #[test]
    fn pointer_status_default_is_pending() {
        assert_eq!(PointerStatus::default(), PointerStatus::Pending);
    }

    #[test]
    fn workflow_status_serde_round_trip() {
        for status in [
            WorkflowStatus::Runnable,
            WorkflowStatus::Suspended,
            WorkflowStatus::Complete,
            WorkflowStatus::Terminated,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: WorkflowStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn pointer_status_serde_round_trip() {
        for status in [
            PointerStatus::Pending,
            PointerStatus::Running,
            PointerStatus::Complete,
            PointerStatus::Skipped,
            PointerStatus::Sleeping,
            PointerStatus::WaitingForEvent,
            PointerStatus::Failed,
            PointerStatus::Compensated,
            PointerStatus::Cancelled,
            PointerStatus::PendingPredecessor,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let deserialized: PointerStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, deserialized);
        }
    }
}
