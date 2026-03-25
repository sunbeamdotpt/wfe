use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::status::PointerStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPointer {
    pub id: String,
    pub step_id: usize,
    pub active: bool,
    pub status: PointerStatus,
    pub sleep_until: Option<DateTime<Utc>>,
    pub persistence_data: Option<serde_json::Value>,
    pub start_time: Option<DateTime<Utc>>,
    pub end_time: Option<DateTime<Utc>>,
    pub event_name: Option<String>,
    pub event_key: Option<String>,
    pub event_published: bool,
    pub event_data: Option<serde_json::Value>,
    pub step_name: Option<String>,
    pub retry_count: u32,
    pub children: Vec<String>,
    pub context_item: Option<serde_json::Value>,
    pub predecessor_id: Option<String>,
    pub outcome: Option<serde_json::Value>,
    pub scope: Vec<String>,
    pub extension_attributes: HashMap<String, serde_json::Value>,
}

impl ExecutionPointer {
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
