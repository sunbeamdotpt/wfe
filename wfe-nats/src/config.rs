use serde::{Deserialize, Serialize};

/// Configuration for the NATS WFE backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    /// NATS server URL, e.g. `nats://localhost:4222`.
    pub url: String,
    /// Prefix applied to streams, subjects, KV buckets, and consumers.
    #[serde(default = "default_prefix")]
    pub prefix: String,
    /// Authentication configuration.
    #[serde(default)]
    pub auth: NatsAuthConfig,
    /// Base name for JetStream streams.
    #[serde(default = "default_stream_name")]
    pub stream_name: String,
    /// Base name for JetStream consumers.
    #[serde(default = "default_consumer_name")]
    pub consumer_name: String,
    /// JetStream KV bucket used for distributed locks.
    #[serde(default = "default_kv_bucket")]
    pub kv_bucket: String,
    /// Message acknowledgement wait duration in seconds.
    #[serde(default = "default_ack_wait_secs")]
    pub ack_wait_secs: u64,
    /// Lock TTL in seconds.
    #[serde(default = "default_lock_ttl_secs")]
    pub lock_ttl_secs: u64,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: "nats://localhost:4222".to_string(),
            prefix: default_prefix(),
            auth: NatsAuthConfig::default(),
            stream_name: default_stream_name(),
            consumer_name: default_consumer_name(),
            kv_bucket: default_kv_bucket(),
            ack_wait_secs: default_ack_wait_secs(),
            lock_ttl_secs: default_lock_ttl_secs(),
        }
    }
}

fn default_prefix() -> String {
    "wfe".to_string()
}

fn default_stream_name() -> String {
    "wfe".to_string()
}

fn default_consumer_name() -> String {
    "wfe".to_string()
}

fn default_kv_bucket() -> String {
    "wfe_locks".to_string()
}

fn default_ack_wait_secs() -> u64 {
    30
}

fn default_lock_ttl_secs() -> u64 {
    30
}

/// NATS authentication configuration.
///
/// Covers the client side of NATS callout auth: the client presents credentials
/// that the NATS server's external auth callout service validates.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NatsAuthConfig {
    /// No authentication.
    #[default]
    None,
    /// Path to a `.creds` file (JWT + nkey seed).
    Credentials {
        /// Path to the credentials file.
        path: String,
    },
    /// JWT and nkey seed pair.
    Jwt {
        /// JWT.
        jwt: String,
        /// Nkey seed.
        nkey: String,
    },
    /// Username and password.
    UserPassword {
        /// Username.
        user: String,
        /// Password.
        password: String,
    },
    /// Auth token.
    Token {
        /// Token.
        token: String,
    },
}
