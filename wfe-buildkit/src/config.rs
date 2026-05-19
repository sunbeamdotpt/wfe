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
    /// Output type: "image", "docker", "oci", "local".
    pub output_type: Option<String>,
    /// Destination path for non-image exports (e.g. tar file or local directory).
    pub output_dest: Option<String>,
    /// BuildKit daemon address.
    #[serde(default = "default_buildkit_addr")]
    pub buildkit_addr: String,
    /// TLS configuration for the BuildKit connection.
    #[serde(default)]
    pub tls: TlsConfig,
    /// Registry authentication credentials keyed by registry host.
    #[serde(default)]
    pub registry_auth: HashMap<String, RegistryAuth>,
    /// Artifact inputs to mount before building.
    /// Map of artifact name → mount point path.
    pub inputs: Option<HashMap<String, String>>,
    /// Workflow data key containing an artifact ref to use as build context.
    /// When set, the artifact is extracted and replaces `context` for this build.
    pub input: Option<String>,
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
    /// Username.
    pub username: String,
    /// Password.
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
            output_dest: Some("/tmp/image.tar".to_string()),
            buildkit_addr: "tcp://buildkitd:1234".to_string(),
            tls: TlsConfig {
                ca: Some("/certs/ca.pem".to_string()),
                cert: Some("/certs/cert.pem".to_string()),
                key: Some("/certs/key.pem".to_string()),
            },
            registry_auth,
            inputs: None,
            input: None,
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
        assert_eq!(config.output_dest, deserialized.output_dest);
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

    #[test]
    fn serde_custom_addr() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "buildkit_addr": "tcp://remote:1234"
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.buildkit_addr, "tcp://remote:1234");
    }

    #[test]
    fn serde_with_timeout() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "timeout_ms": 60000
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.timeout_ms, Some(60000));
    }

    #[test]
    fn serde_with_tags_and_push() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "tags": ["myapp:latest", "myapp:v1.0"],
            "push": true
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.tags, vec!["myapp:latest", "myapp:v1.0"]);
        assert!(config.push);
    }

    #[test]
    fn serde_with_build_args() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "build_args": {"VERSION": "1.0", "DEBUG": "false"}
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.build_args.len(), 2);
        assert_eq!(config.build_args["VERSION"], "1.0");
        assert_eq!(config.build_args["DEBUG"], "false");
    }

    #[test]
    fn serde_with_cache_config() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "cache_from": ["type=registry,ref=cache:latest"],
            "cache_to": ["type=registry,ref=cache:latest,mode=max"]
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.cache_from.len(), 1);
        assert_eq!(config.cache_to.len(), 1);
    }

    #[test]
    fn serde_with_output_type() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "output_type": "tar"
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.output_type, Some("tar".to_string()));
    }

    #[test]
    fn serde_with_registry_auth() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "registry_auth": {
                "ghcr.io": {"username": "bot", "password": "tok"},
                "docker.io": {"username": "u", "password": "p"}
            }
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.registry_auth.len(), 2);
        assert_eq!(config.registry_auth["ghcr.io"].username, "bot");
        assert_eq!(config.registry_auth["docker.io"].password, "p");
    }

    #[test]
    fn serde_with_tls() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "tls": {
                "ca": "/certs/ca.pem",
                "cert": "/certs/cert.pem",
                "key": "/certs/key.pem"
            }
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.tls.ca, Some("/certs/ca.pem".to_string()));
        assert_eq!(config.tls.cert, Some("/certs/cert.pem".to_string()));
        assert_eq!(config.tls.key, Some("/certs/key.pem".to_string()));
    }

    #[test]
    fn serde_partial_tls() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "tls": {"ca": "/certs/ca.pem"}
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.tls.ca, Some("/certs/ca.pem".to_string()));
        assert_eq!(config.tls.cert, None);
        assert_eq!(config.tls.key, None);
    }

    #[test]
    fn serde_empty_tls_object() {
        let json = r#"{
            "dockerfile": "Dockerfile",
            "context": ".",
            "tls": {}
        }"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.tls.ca, None);
        assert_eq!(config.tls.cert, None);
        assert_eq!(config.tls.key, None);
    }

    #[test]
    fn tls_config_clone() {
        let tls = TlsConfig {
            ca: Some("ca".to_string()),
            cert: Some("cert".to_string()),
            key: Some("key".to_string()),
        };
        let cloned = tls.clone();
        assert_eq!(tls.ca, cloned.ca);
        assert_eq!(tls.cert, cloned.cert);
        assert_eq!(tls.key, cloned.key);
    }

    #[test]
    fn tls_config_debug() {
        let tls = TlsConfig::default();
        let debug = format!("{:?}", tls);
        assert!(debug.contains("TlsConfig"));
    }

    #[test]
    fn buildkit_config_debug() {
        let json = r#"{"dockerfile": "Dockerfile", "context": "."}"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        let debug = format!("{:?}", config);
        assert!(debug.contains("BuildkitConfig"));
    }

    #[test]
    fn registry_auth_clone() {
        let auth = RegistryAuth {
            username: "u".to_string(),
            password: "p".to_string(),
        };
        let cloned = auth.clone();
        assert_eq!(auth.username, cloned.username);
        assert_eq!(auth.password, cloned.password);
    }

    #[test]
    fn buildkit_config_clone() {
        let json = r#"{"dockerfile": "Dockerfile", "context": "."}"#;
        let config: BuildkitConfig = serde_json::from_str(json).unwrap();
        let cloned = config.clone();
        assert_eq!(config.dockerfile, cloned.dockerfile);
        assert_eq!(config.context, cloned.context);
        assert_eq!(config.buildkit_addr, cloned.buildkit_addr);
    }
}
