use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::poll_config::PollEndpointConfig;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Executionresult.
pub struct ExecutionResult {
    /// Whether the workflow should proceed to the next step.
    pub proceed: bool,
    /// Outcome value for decision-based routing.
    pub outcome_value: Option<serde_json::Value>,
    /// Duration to sleep before re-executing.
    #[serde(default, with = "super::option_duration_millis")]
    pub sleep_for: Option<Duration>,
    /// Step-specific state to persist between executions.
    pub persistence_data: Option<serde_json::Value>,
    /// Event name to wait for.
    pub event_name: Option<String>,
    /// Event key to match.
    pub event_key: Option<String>,
    /// Only consider events published after this time.
    pub event_as_of: Option<DateTime<Utc>>,
    /// Values to branch execution on (for ForEach/Parallel).
    pub branch_values: Option<Vec<serde_json::Value>>,
    /// Poll endpoint configuration for external service polling.
    pub poll_endpoint: Option<PollEndpointConfig>,
    /// Output data to merge into workflow.data after step completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_data: Option<serde_json::Value>,
}

impl ExecutionResult {
    /// Continue to the next step.
    pub fn next() -> Self {
        Self {
            proceed: true,
            ..Default::default()
        }
    }

    /// Continue with an outcome value for decision routing.
    pub fn outcome(value: impl Into<serde_json::Value>) -> Self {
        Self {
            proceed: true,
            outcome_value: Some(value.into()),
            ..Default::default()
        }
    }

    /// Pause execution and persist step-specific data.
    pub fn persist(data: serde_json::Value) -> Self {
        Self {
            proceed: false,
            persistence_data: Some(data),
            ..Default::default()
        }
    }

    /// Create child branches for parallel/foreach execution.
    pub fn branch(
        values: Vec<serde_json::Value>,
        persistence_data: Option<serde_json::Value>,
    ) -> Self {
        Self {
            proceed: false,
            branch_values: Some(values),
            persistence_data,
            ..Default::default()
        }
    }

    /// Sleep for a duration before re-executing.
    pub fn sleep(duration: Duration, persistence_data: Option<serde_json::Value>) -> Self {
        Self {
            proceed: false,
            sleep_for: Some(duration),
            persistence_data,
            ..Default::default()
        }
    }

    /// Wait for an external event.
    pub fn wait_for_event(
        event_name: impl Into<String>,
        event_key: impl Into<String>,
        as_of: DateTime<Utc>,
    ) -> Self {
        Self {
            proceed: false,
            event_name: Some(event_name.into()),
            event_key: Some(event_key.into()),
            event_as_of: Some(as_of),
            ..Default::default()
        }
    }

    /// Poll an external endpoint until a condition is met.
    pub fn poll_endpoint(config: PollEndpointConfig) -> Self {
        Self {
            proceed: false,
            poll_endpoint: Some(config),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn next_proceeds_with_no_data() {
        let result = ExecutionResult::next();
        assert!(result.proceed);
        assert!(result.outcome_value.is_none());
        assert!(result.sleep_for.is_none());
        assert!(result.persistence_data.is_none());
        assert!(result.event_name.is_none());
        assert!(result.branch_values.is_none());
        assert!(result.poll_endpoint.is_none());
    }

    #[test]
    fn outcome_proceeds_with_value() {
        let result = ExecutionResult::outcome(serde_json::json!(42));
        assert!(result.proceed);
        assert_eq!(result.outcome_value, Some(serde_json::json!(42)));
    }

    #[test]
    fn persist_does_not_proceed() {
        let data = serde_json::json!({"counter": 5});
        let result = ExecutionResult::persist(data.clone());
        assert!(!result.proceed);
        assert_eq!(result.persistence_data, Some(data));
    }

    #[test]
    fn branch_creates_child_values() {
        let values = vec![
            serde_json::json!(1),
            serde_json::json!(2),
            serde_json::json!(3),
        ];
        let result = ExecutionResult::branch(values.clone(), None);
        assert!(!result.proceed);
        assert_eq!(result.branch_values, Some(values));
    }

    #[test]
    fn sleep_sets_duration() {
        let result = ExecutionResult::sleep(Duration::from_secs(30), None);
        assert!(!result.proceed);
        assert_eq!(result.sleep_for, Some(Duration::from_secs(30)));
    }

    #[test]
    fn wait_for_event_sets_event_fields() {
        let now = Utc::now();
        let result = ExecutionResult::wait_for_event("order.completed", "order-123", now);
        assert!(!result.proceed);
        assert_eq!(result.event_name.as_deref(), Some("order.completed"));
        assert_eq!(result.event_key.as_deref(), Some("order-123"));
        assert_eq!(result.event_as_of, Some(now));
    }

    #[test]
    fn poll_endpoint_sets_config() {
        use super::super::poll_config::*;
        use std::collections::HashMap;

        let config = PollEndpointConfig {
            url: "https://api.example.com/status".into(),
            method: HttpMethod::Get,
            headers: HashMap::new(),
            body: None,
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(300),
            condition: PollCondition::StatusCode(200),
        };
        let result = ExecutionResult::poll_endpoint(config.clone());
        assert!(!result.proceed);
        assert_eq!(result.poll_endpoint, Some(config));
    }

    #[test]
    fn serde_round_trip() {
        let result =
            ExecutionResult::sleep(Duration::from_secs(30), Some(serde_json::json!({"x": 1})));
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: ExecutionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(result.proceed, deserialized.proceed);
        assert_eq!(result.sleep_for, deserialized.sleep_for);
        assert_eq!(result.persistence_data, deserialized.persistence_data);
    }
}
