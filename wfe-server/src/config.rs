use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use serde::Deserialize;

/// WFE workflow server.
#[derive(Parser, Debug)]
#[command(name = "wfe-server", version, about)]
pub struct Cli {
    /// Config file path.
    #[arg(short, long, default_value = "wfe-server.toml")]
    pub config: PathBuf,

    /// gRPC listen address.
    #[arg(long, env = "WFE_GRPC_ADDR")]
    pub grpc_addr: Option<SocketAddr>,

    /// HTTP listen address (webhooks).
    #[arg(long, env = "WFE_HTTP_ADDR")]
    pub http_addr: Option<SocketAddr>,

    /// Persistence backend: sqlite or postgres.
    #[arg(long, env = "WFE_PERSISTENCE")]
    pub persistence: Option<String>,

    /// Database URL or path.
    #[arg(long, env = "WFE_DB_URL")]
    pub db_url: Option<String>,

    /// Queue backend: memory or valkey.
    #[arg(long, env = "WFE_QUEUE")]
    pub queue: Option<String>,

    /// Queue URL (for valkey).
    #[arg(long, env = "WFE_QUEUE_URL")]
    pub queue_url: Option<String>,

    /// OpenSearch URL (enables log + workflow search).
    #[arg(long, env = "WFE_SEARCH_URL")]
    pub search_url: Option<String>,

    /// Directory to auto-load YAML workflow definitions from.
    #[arg(long, env = "WFE_WORKFLOWS_DIR")]
    pub workflows_dir: Option<PathBuf>,

    /// Comma-separated bearer tokens for API auth.
    #[arg(long, env = "WFE_AUTH_TOKENS")]
    pub auth_tokens: Option<String>,
}

/// Server configuration (deserialized from TOML).
#[derive(Debug, Deserialize, Clone)]
#[serde(default)]
pub struct ServerConfig {
    pub grpc_addr: SocketAddr,
    pub http_addr: SocketAddr,
    pub persistence: PersistenceConfig,
    pub queue: QueueConfig,
    pub search: Option<SearchConfig>,
    pub auth: AuthConfig,
    pub webhook: WebhookConfig,
    pub workflows_dir: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            grpc_addr: "0.0.0.0:50051".parse().unwrap(),
            http_addr: "0.0.0.0:8080".parse().unwrap(),
            persistence: PersistenceConfig::default(),
            queue: QueueConfig::default(),
            search: None,
            auth: AuthConfig::default(),
            webhook: WebhookConfig::default(),
            workflows_dir: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "backend")]
pub enum PersistenceConfig {
    #[serde(rename = "sqlite")]
    Sqlite { path: String },
    #[serde(rename = "postgres")]
    Postgres { url: String },
}

impl Default for PersistenceConfig {
    fn default() -> Self {
        Self::Sqlite {
            path: "wfe.db".to_string(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "backend")]
pub enum QueueConfig {
    #[serde(rename = "memory")]
    InMemory,
    #[serde(rename = "valkey")]
    Valkey { url: String },
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self::InMemory
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct SearchConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct AuthConfig {
    /// Static bearer tokens (simple auth, no OIDC needed).
    #[serde(default)]
    pub tokens: Vec<String>,
    /// OIDC issuer URL (e.g., https://auth.example.com/realms/myapp).
    /// Enables JWT validation via OIDC discovery + JWKS.
    #[serde(default)]
    pub oidc_issuer: Option<String>,
    /// Expected JWT audience claim.
    #[serde(default)]
    pub oidc_audience: Option<String>,
    /// Webhook HMAC secrets per source.
    #[serde(default)]
    pub webhook_secrets: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WebhookConfig {
    #[serde(default)]
    pub triggers: Vec<WebhookTrigger>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebhookTrigger {
    pub source: String,
    pub event: String,
    #[serde(default)]
    pub match_ref: Option<String>,
    pub workflow_id: String,
    pub version: u32,
    #[serde(default)]
    pub data_mapping: HashMap<String, String>,
}

/// Load configuration with layered overrides: CLI > env > file.
pub fn load(cli: &Cli) -> ServerConfig {
    let mut config = if cli.config.exists() {
        let content = std::fs::read_to_string(&cli.config)
            .unwrap_or_else(|e| panic!("failed to read config file {}: {e}", cli.config.display()));
        toml::from_str(&content)
            .unwrap_or_else(|e| panic!("failed to parse config file {}: {e}", cli.config.display()))
    } else {
        ServerConfig::default()
    };

    if let Some(addr) = cli.grpc_addr {
        config.grpc_addr = addr;
    }
    if let Some(addr) = cli.http_addr {
        config.http_addr = addr;
    }
    if let Some(ref dir) = cli.workflows_dir {
        config.workflows_dir = Some(dir.clone());
    }

    // Persistence override.
    if let Some(ref backend) = cli.persistence {
        let url = cli
            .db_url
            .clone()
            .unwrap_or_else(|| "wfe.db".to_string());
        config.persistence = match backend.as_str() {
            "postgres" => PersistenceConfig::Postgres { url },
            _ => PersistenceConfig::Sqlite { path: url },
        };
    } else if let Some(ref url) = cli.db_url {
        // Infer backend from URL.
        if url.starts_with("postgres") {
            config.persistence = PersistenceConfig::Postgres { url: url.clone() };
        } else {
            config.persistence = PersistenceConfig::Sqlite { path: url.clone() };
        }
    }

    // Queue override.
    if let Some(ref backend) = cli.queue {
        config.queue = match backend.as_str() {
            "valkey" | "redis" => {
                let url = cli
                    .queue_url
                    .clone()
                    .unwrap_or_else(|| "redis://127.0.0.1:6379".to_string());
                QueueConfig::Valkey { url }
            }
            _ => QueueConfig::InMemory,
        };
    }

    // Search override.
    if let Some(ref url) = cli.search_url {
        config.search = Some(SearchConfig { url: url.clone() });
    }

    // Auth tokens override.
    if let Some(ref tokens) = cli.auth_tokens {
        config.auth.tokens = tokens
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = ServerConfig::default();
        assert_eq!(config.grpc_addr, "0.0.0.0:50051".parse().unwrap());
        assert_eq!(config.http_addr, "0.0.0.0:8080".parse().unwrap());
        assert!(matches!(config.persistence, PersistenceConfig::Sqlite { .. }));
        assert!(matches!(config.queue, QueueConfig::InMemory));
        assert!(config.search.is_none());
        assert!(config.auth.tokens.is_empty());
        assert!(config.webhook.triggers.is_empty());
    }

    #[test]
    fn parse_toml_config() {
        let toml = r#"
grpc_addr = "127.0.0.1:9090"
http_addr = "127.0.0.1:8081"

[persistence]
backend = "postgres"
url = "postgres://localhost/wfe"

[queue]
backend = "valkey"
url = "redis://localhost:6379"

[search]
url = "http://localhost:9200"

[auth]
tokens = ["token1", "token2"]

[auth.webhook_secrets]
github = "mysecret"

[[webhook.triggers]]
source = "github"
event = "push"
match_ref = "refs/heads/main"
workflow_id = "ci"
version = 1
"#;
        let config: ServerConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.grpc_addr, "127.0.0.1:9090".parse().unwrap());
        assert!(matches!(config.persistence, PersistenceConfig::Postgres { .. }));
        assert!(matches!(config.queue, QueueConfig::Valkey { .. }));
        assert!(config.search.is_some());
        assert_eq!(config.auth.tokens.len(), 2);
        assert_eq!(config.auth.webhook_secrets.get("github").unwrap(), "mysecret");
        assert_eq!(config.webhook.triggers.len(), 1);
        assert_eq!(config.webhook.triggers[0].workflow_id, "ci");
    }

    #[test]
    fn cli_overrides_file() {
        let cli = Cli {
            config: PathBuf::from("/nonexistent"),
            grpc_addr: Some("127.0.0.1:9999".parse().unwrap()),
            http_addr: None,
            persistence: Some("postgres".to_string()),
            db_url: Some("postgres://db/wfe".to_string()),
            queue: Some("valkey".to_string()),
            queue_url: Some("redis://valkey:6379".to_string()),
            search_url: Some("http://os:9200".to_string()),
            workflows_dir: Some(PathBuf::from("/workflows")),
            auth_tokens: Some("tok1, tok2".to_string()),
        };
        let config = load(&cli);
        assert_eq!(config.grpc_addr, "127.0.0.1:9999".parse().unwrap());
        assert!(matches!(config.persistence, PersistenceConfig::Postgres { ref url } if url == "postgres://db/wfe"));
        assert!(matches!(config.queue, QueueConfig::Valkey { ref url } if url == "redis://valkey:6379"));
        assert_eq!(config.search.unwrap().url, "http://os:9200");
        assert_eq!(config.workflows_dir.unwrap(), PathBuf::from("/workflows"));
        assert_eq!(config.auth.tokens, vec!["tok1", "tok2"]);
    }

    #[test]
    fn infer_postgres_from_url() {
        let cli = Cli {
            config: PathBuf::from("/nonexistent"),
            grpc_addr: None,
            http_addr: None,
            persistence: None,
            db_url: Some("postgres://localhost/wfe".to_string()),
            queue: None,
            queue_url: None,
            search_url: None,
            workflows_dir: None,
            auth_tokens: None,
        };
        let config = load(&cli);
        assert!(matches!(config.persistence, PersistenceConfig::Postgres { .. }));
    }

    // ── Security regression tests ──

    #[test]
    #[should_panic(expected = "failed to parse config file")]
    fn security_malformed_config_panics() {
        // HIGH-19: Malformed config must NOT silently fall back to defaults.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "this is not { valid toml @@@@").unwrap();
        let cli = Cli {
            config: tmp.path().to_path_buf(),
            grpc_addr: None,
            http_addr: None,
            persistence: None,
            db_url: None,
            queue: None,
            queue_url: None,
            search_url: None,
            workflows_dir: None,
            auth_tokens: None,
        };
        load(&cli);
    }

    #[test]
    fn trigger_data_mapping() {
        let toml = r#"
[[triggers]]
source = "github"
event = "push"
workflow_id = "ci"
version = 1

[triggers.data_mapping]
repo = "$.repository.full_name"
commit = "$.head_commit.id"
"#;
        let config: WebhookConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.triggers[0].data_mapping.len(), 2);
        assert_eq!(config.triggers[0].data_mapping["repo"], "$.repository.full_name");
    }
}
