use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Configuration for a BuildKit image build step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildkitConfig {
    /// Path to the Dockerfile (or directory containing it).
    pub dockerfile: String,
    /// Build context directory.
    pub context: String,
    /// Multi-stage build target.
    pub target: Option<String>,
    /// Image tags to apply.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Build arguments passed as `--opt build-arg:KEY=VALUE`.
    #[serde(default)]
    pub build_args: HashMap<String, String>,
    /// Cache import sources.
    #[serde(default)]
    pub cache_from: Vec<String>,
    /// Cache export destinations.
    #[serde(default)]
    pub cache_to: Vec<String>,
    /// Whether to push the built image.
    #[serde(default)]
    pub push: bool,
    /// Output type: "image", "local", "tar".
    pub output_type: Option<String>,
    /// BuildKit daemon address.
    #[serde(default = "default_buildkit_addr")]
    pub buildkit_addr: String,
    /// TLS configuration for the BuildKit connection.
    #[serde(default)]
    pub tls: TlsConfig,
    /// Registry authentication credentials keyed by registry host.
    #[serde(default)]
    pub registry_auth: HashMap<String, RegistryAuth>,
    /// Execution timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// TLS certificate paths for securing the BuildKit connection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    /// Path to the CA certificate.
    pub ca: Option<String>,
    /// Path to the client certificate.
    pub cert: Option<String>,
    /// Path to the client private key.
    pub key: Option<String>,
}

/// Credentials for authenticating with a container registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAuth {
    pub username: String,
    pub password: String,
}

fn default_buildkit_addr() -> String {
    "unix:///run/buildkit/buildkitd.sock".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn serde_round_trip_full_config() {
        let mut build_args = HashMap::new();
        build_args.insert("RUST_VERSION".to_string(), "1.78".to_string());

        let mut registry_auth = HashMap::new();
        registry_auth.insert(
            "ghcr.io".to_string(),
            RegistryAuth {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
        );

        let config = BuildkitConfig {
            dockerfile: "./Dockerfile".to_string(),
            context: ".".to_string(),
            target: Some("runtime".to_string()),
            tags: vec!["myapp:latest".to_string(), "myapp:v1.0".to_string()],
            build_args,
            cache_from: vec!["type=registry,ref=myapp:cache".to_string()],
            cache_to: vec!["type=registry,ref=myapp:cache,mode=max".to_string()],
            push: true,
            output_type: Some("image".to_string()),
            buildkit_addr: "tcp://buildkitd:1234".to_string(),
            tls: TlsConfig {
                ca: Some("/certs/ca.pem".to_string()),
                cert: Some("/certs/cert.pem".to_string()),
                key: Some("/certs/key.pem".to_string()),
            },
            registry_auth,
            timeout_ms: Some(300_000),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: BuildkitConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config.dockerfile, deserialized.dockerfile);
        assert_eq!(config.context, deserialized.context);
        assert_eq!(config.target, deserialized.target);
        assert_eq!(config.tags, deserialized.tags);
        assert_eq!(config.build_args, deserialized.build_args);
        assert_eq!(config.cache_from, deserialized.cache_from);
        assert_eq!(config.cache_to, deserialized.cache_to);
        assert_eq!(config.push, deserialized.push);
        assert_eq!(config.output_type, deserialized.output_type);
        assert_eq!(config.buildkit_addr, deserialized.buildkit_addr);
        assert_eq!(config.tls.ca, deserialized.tls.ca);
        assert_eq!(config.tls.cert, deserialized.tls.cert);
        assert_eq!(config.tls.key, deserialized.tls.key);
        assert_eq!(config.timeout_ms, deserialized.timeout_ms);
    }

    #[test]
    fn serde_round_trip_minimal_config() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": "."
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.dockerfile, "Dockerfile");
        assert_eq!(config.context, ".");
        assert_eq!(config.target, None);
        assert!(config.tags.is_empty());
        assert!(config.build_args.is_empty());
        assert!(config.cache_from.is_empty());
        assert!(config.cache_to.is_empty());
        assert!(!config.push);
        assert_eq!(config.output_type, None);
        assert_eq!(config.buildkit_addr, "unix:///run/buildkit/buildkitd.sock");
        assert_eq!(config.timeout_ms, None);

        // Round-trip
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: BuildkitConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config.dockerfile, deserialized.dockerfile);
        assert_eq!(config.context, deserialized.context);
    }

    #[test]
    fn default_buildkit_addr_value() {
        let addr = default_buildkit_addr();
        assert_eq!(addr, "unix:///run/buildkit/buildkitd.sock");
    }

    #[test]
    fn tls_config_defaults_to_none() {
        let tls = TlsConfig::default();
        assert_eq!(tls.ca, None);
        assert_eq!(tls.cert, None);
        assert_eq!(tls.key, None);
    }

    #[test]
    fn registry_auth_serde() {
        let auth = RegistryAuth {
            username: "admin".to_string(),
            password: "secret123".to_string(),
        };

        let json = serde_json::to_string(&auth).unwrap();
        let deserialized: RegistryAuth = serde_json::from_str(&json).unwrap();

        assert_eq!(auth.username, deserialized.username);
        assert_eq!(auth.password, deserialized.password);
    }
}
