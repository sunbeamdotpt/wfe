use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
/// Httpmethod.
pub enum HttpMethod {
    #[default]
    /// Get.
    Get,
    /// Post.
    Post,
    /// Put.
    Put,
    /// Patch.
    Patch,
    /// Delete.
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Pollcondition.
pub enum PollCondition {
    /// Check a JSON path equals a value: e.g. JsonPathEquals("$.status", "complete")
    JsonPathEquals {
        /// JSON path expression.
        path: String,
        /// Expected value at the path.
        value: serde_json::Value,
    },
    /// Check HTTP status code
    StatusCode(u16),
    /// Check response body contains string
    BodyContains(String),
}

impl Default for PollCondition {
    fn default() -> Self {
        Self::StatusCode(200)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
/// Pollendpointconfig.
pub struct PollEndpointConfig {
    /// URL template. Supports `{placeholder}` interpolation from workflow data.
    pub url: String,
    /// Method.
    pub method: HttpMethod,
    #[serde(default)]
    /// Headers.
    pub headers: HashMap<String, String>,
    #[serde(default)]
    /// Body.
    pub body: Option<serde_json::Value>,
    #[serde(with = "super::duration_millis")]
    /// Interval.
    pub interval: Duration,
    #[serde(with = "super::duration_millis")]
    /// Timeout.
    pub timeout: Duration,
    /// Condition.
    pub condition: PollCondition,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn poll_config_serde_round_trip() {
        let config = PollEndpointConfig {
            url: "https://api.example.com/status/{id}".into(),
            method: HttpMethod::Get,
            headers: HashMap::from([("Authorization".into(), "Bearer token123".into())]),
            body: None,
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(3600),
            condition: PollCondition::JsonPathEquals {
                path: "$.status".into(),
                value: serde_json::Value::String("complete".into()),
            },
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: PollEndpointConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn poll_condition_status_code() {
        let cond = PollCondition::StatusCode(200);
        let json = serde_json::to_string(&cond).unwrap();
        let deserialized: PollCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, deserialized);
    }

    #[test]
    fn poll_condition_body_contains() {
        let cond = PollCondition::BodyContains("success".into());
        let json = serde_json::to_string(&cond).unwrap();
        let deserialized: PollCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, deserialized);
    }
}
