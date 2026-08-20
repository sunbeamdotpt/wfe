use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::status::PointerStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Executionpointer.
pub struct ExecutionPointer {
    /// Id.
    pub id: String,
    /// Step id.
    pub step_id: usize,
    /// Active.
    pub active: bool,
    /// Status.
    pub status: PointerStatus,
    /// Sleep until.
    pub sleep_until: Option<DateTime<Utc>>,
    /// Persistence data.
    pub persistence_data: Option<serde_json::Value>,
    /// Start time.
    pub start_time: Option<DateTime<Utc>>,
    /// End time.
    pub end_time: Option<DateTime<Utc>>,
    /// Event name.
    pub event_name: Option<String>,
    /// Event key.
    pub event_key: Option<String>,
    /// Event published.
    pub event_published: bool,
    /// Event data.
    pub event_data: Option<serde_json::Value>,
    /// Step name.
    pub step_name: Option<String>,
    /// Retry count.
    pub retry_count: u32,
    /// Children.
    pub children: Vec<String>,
    /// Context item.
    pub context_item: Option<serde_json::Value>,
    /// Predecessor id.
    pub predecessor_id: Option<String>,
    /// Outcome.
    pub outcome: Option<serde_json::Value>,
    /// Scope.
    pub scope: Vec<String>,
    /// Extension attributes.
    pub extension_attributes: HashMap<String, serde_json::Value>,
}

impl ExecutionPointer {
    /// Create a new active pointer for the given step id.
    pub fn new(step_id: usize) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            step_id,
            active: true,
            status: PointerStatus::Pending,
            sleep_until: None,
            persistence_data: None,
            start_time: None,
            end_time: None,
            event_name: None,
            event_key: None,
            event_published: false,
            event_data: None,
            step_name: None,
            retry_count: 0,
            children: Vec::new(),
            context_item: None,
            predecessor_id: None,
            outcome: None,
            scope: Vec::new(),
            extension_attributes: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn new_pointer_has_correct_defaults() {
        let pointer = ExecutionPointer::new(0);
        assert_eq!(pointer.step_id, 0);
        assert!(pointer.active);
        assert_eq!(pointer.status, PointerStatus::Pending);
        assert_eq!(pointer.retry_count, 0);
        assert!(pointer.children.is_empty());
        assert!(pointer.scope.is_empty());
        assert!(!pointer.event_published);
    }

    #[test]
    fn new_pointer_generates_unique_ids() {
        let p1 = ExecutionPointer::new(0);
        let p2 = ExecutionPointer::new(0);
        assert_ne!(p1.id, p2.id);
    }

    #[test]
    fn serde_round_trip() {
        let mut pointer = ExecutionPointer::new(3);
        pointer.status = PointerStatus::Running;
        pointer.retry_count = 2;
        pointer.persistence_data = Some(serde_json::json!({"step_state": true}));
        pointer.children = vec!["child-1".into(), "child-2".into()];

        let json = serde_json::to_string(&pointer).unwrap();
        let deserialized: ExecutionPointer = serde_json::from_str(&json).unwrap();

        assert_eq!(pointer.id, deserialized.id);
        assert_eq!(pointer.step_id, deserialized.step_id);
        assert_eq!(pointer.status, deserialized.status);
        assert_eq!(pointer.retry_count, deserialized.retry_count);
        assert_eq!(pointer.persistence_data, deserialized.persistence_data);
        assert_eq!(pointer.children, deserialized.children);
    }
}
