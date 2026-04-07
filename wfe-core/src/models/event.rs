use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub event_name: String,
    pub event_key: String,
    pub event_data: serde_json::Value,
    pub event_time: DateTime<Utc>,
    pub is_processed: bool,
}

impl Event {
    pub fn new(
        event_name: impl Into<String>,
        event_key: impl Into<String>,
        event_data: serde_json::Value,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            event_name: event_name.into(),
            event_key: event_key.into(),
            event_data,
            event_time: Utc::now(),
            is_processed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSubscription {
    pub id: String,
    pub workflow_id: String,
    pub step_id: usize,
    pub execution_pointer_id: String,
    pub event_name: String,
    pub event_key: String,
    pub subscribe_as_of: DateTime<Utc>,
    pub subscription_data: Option<serde_json::Value>,
    pub external_token: Option<String>,
    pub external_worker_id: Option<String>,
    pub external_token_expiry: Option<DateTime<Utc>>,
}

impl EventSubscription {
    pub fn new(
        workflow_id: impl Into<String>,
        step_id: usize,
        execution_pointer_id: impl Into<String>,
        event_name: impl Into<String>,
        event_key: impl Into<String>,
        subscribe_as_of: DateTime<Utc>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            workflow_id: workflow_id.into(),
            step_id,
            execution_pointer_id: execution_pointer_id.into(),
            event_name: event_name.into(),
            event_key: event_key.into(),
            subscribe_as_of,
            subscription_data: None,
            external_token: None,
            external_worker_id: None,
            external_token_expiry: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn new_event_defaults() {
        let event = Event::new(
            "order.created",
            "order-456",
            serde_json::json!({"amount": 100}),
        );
        assert_eq!(event.event_name, "order.created");
        assert_eq!(event.event_key, "order-456");
        assert!(!event.is_processed);
    }

    #[test]
    fn new_event_generates_unique_ids() {
        let e1 = Event::new("test", "key", serde_json::json!(null));
        let e2 = Event::new("test", "key", serde_json::json!(null));
        assert_ne!(e1.id, e2.id);
    }

    #[test]
    fn event_serde_round_trip() {
        let event = Event::new("test", "key", serde_json::json!({"data": true}));
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event.id, deserialized.id);
        assert_eq!(event.event_name, deserialized.event_name);
        assert_eq!(event.event_data, deserialized.event_data);
    }

    #[test]
    fn new_subscription_defaults() {
        let sub = EventSubscription::new("wf-1", 0, "ptr-1", "evt", "key", Utc::now());
        assert_eq!(sub.workflow_id, "wf-1");
        assert_eq!(sub.step_id, 0);
        assert!(sub.external_token.is_none());
        assert!(sub.subscription_data.is_none());
    }

    #[test]
    fn subscription_serde_round_trip() {
        let sub = EventSubscription::new("wf-1", 2, "ptr-1", "evt", "key", Utc::now());
        let json = serde_json::to_string(&sub).unwrap();
        let deserialized: EventSubscription = serde_json::from_str(&json).unwrap();
        assert_eq!(sub.id, deserialized.id);
        assert_eq!(sub.workflow_id, deserialized.workflow_id);
        assert_eq!(sub.event_name, deserialized.event_name);
    }
}
