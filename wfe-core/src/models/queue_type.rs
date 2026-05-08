use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
/// Queuetype.
pub enum QueueType {
    /// Workflow.
    Workflow,
    /// Event.
    Event,
    /// Index.
    Index,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn queue_types_are_distinct() {
        assert_ne!(QueueType::Workflow, QueueType::Event);
        assert_ne!(QueueType::Event, QueueType::Index);
        assert_ne!(QueueType::Workflow, QueueType::Index);
    }

    #[test]
    fn serde_round_trip() {
        for qt in [QueueType::Workflow, QueueType::Event, QueueType::Index] {
            let json = serde_json::to_string(&qt).unwrap();
            let deserialized: QueueType = serde_json::from_str(&json).unwrap();
            assert_eq!(qt, deserialized);
        }
    }
}
