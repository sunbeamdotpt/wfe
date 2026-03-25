use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PollCondition {
    /// Check a JSON path equals a value: e.g. JsonPathEquals("$.status", "complete")
    JsonPathEquals {
        path: String,
        value: serde_json::Value,
    },
    /// Check HTTP status code
    StatusCode(u16),
    /// Check response body contains string
    BodyContains(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollEndpointConfig {
    /// URL template. Supports `{placeholder}` interpolation from workflow data.
    pub url: String,
    pub method: HttpMethod,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub body: Option<serde_json::Value>,
    #[serde(with = "super::duration_millis")]
    pub interval: Duration,
    #[serde(with = "super::duration_millis")]
    pub timeout: Duration,
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
