use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Errorbehavior.
pub enum ErrorBehavior {
    /// Retry.
    Retry {
        /// Delay between retries.
        #[serde(with = "duration_millis")]
        interval: Duration,
        /// Maximum number of retry attempts.
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
    /// Suspend.
    Suspend,
    /// Terminate.
    Terminate,
    /// Compensate.
    Compensate,
}

fn default_max_retries() -> u32 {
    3
}

impl Default for ErrorBehavior {
    fn default() -> Self {
        Self::Retry {
            interval: Duration::from_secs(60),
            max_retries: default_max_retries(),
        }
    }
}

mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

/// Serialize.
    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

/// Deserialize.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn default_is_retry_60s() {
        let behavior = ErrorBehavior::default();
        assert_eq!(
            behavior,
            ErrorBehavior::Retry {
                interval: Duration::from_secs(60),
                max_retries: 3,
            }
        );
    }

    #[test]
    fn serde_round_trip() {
        let variants = vec![
            ErrorBehavior::Retry {
                interval: Duration::from_secs(30),
                max_retries: 3,
            },
            ErrorBehavior::Suspend,
            ErrorBehavior::Terminate,
            ErrorBehavior::Compensate,
        ];
        for variant in variants {
            let json = serde_json::to_string(&variant).unwrap();
            let deserialized: ErrorBehavior = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, deserialized);
        }
    }
}
