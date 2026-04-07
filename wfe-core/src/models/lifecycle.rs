use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LifecycleEvent {
    pub event_time_utc: DateTime<Utc>,
    pub workflow_instance_id: String,
    pub workflow_definition_id: String,
    pub version: u32,
    pub reference: Option<String>,
    pub event_type: LifecycleEventType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleEventType {
    Started,
    Resumed,
    Suspended,
    Completed,
    Terminated,
    Error {
        message: String,
    },
    StepStarted {
        step_id: usize,
        step_name: Option<String>,
    },
    StepCompleted {
        step_id: usize,
        step_name: Option<String>,
    },
}

impl LifecycleEvent {
    pub fn new(
        workflow_instance_id: impl Into<String>,
        workflow_definition_id: impl Into<String>,
        version: u32,
        event_type: LifecycleEventType,
    ) -> Self {
        Self {
            event_time_utc: Utc::now(),
            workflow_instance_id: workflow_instance_id.into(),
            workflow_definition_id: workflow_definition_id.into(),
            version,
            reference: None,
            event_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn lifecycle_event_types_are_distinct() {
        assert_ne!(LifecycleEventType::Started, LifecycleEventType::Completed);
    }

    #[test]
    fn serde_round_trip_simple_variant() {
        let event = LifecycleEvent::new("wf-1", "def-1", 1, LifecycleEventType::Started);
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LifecycleEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(
            event.workflow_instance_id,
            deserialized.workflow_instance_id
        );
        assert_eq!(event.event_type, deserialized.event_type);
    }

    #[test]
    fn serde_round_trip_error_variant() {
        let event = LifecycleEvent::new(
            "wf-1",
            "def-1",
            1,
            LifecycleEventType::Error {
                message: "boom".into(),
            },
        );
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LifecycleEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.event_type,
            LifecycleEventType::Error {
                message: "boom".into()
            }
        );
    }

    #[test]
    fn serde_round_trip_step_variant() {
        let event = LifecycleEvent::new(
            "wf-1",
            "def-1",
            1,
            LifecycleEventType::StepStarted {
                step_id: 3,
                step_name: Some("ProcessOrder".into()),
            },
        );
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: LifecycleEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.event_type,
            LifecycleEventType::StepStarted {
                step_id: 3,
                step_name: Some("ProcessOrder".into()),
            }
        );
    }
}
