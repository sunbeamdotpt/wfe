use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerdConfig {
    pub image: String,
    pub command: Option<Vec<String>>,
    pub run: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub volumes: Vec<VolumeMountConfig>,
    pub working_dir: Option<String>,
    #[serde(default = "default_user")]
    pub user: String,
    #[serde(default = "default_network")]
    pub network: String,
    pub memory: Option<String>,
    pub cpu: Option<String>,
    #[serde(default = "default_pull")]
    pub pull: String,
    #[serde(default = "default_containerd_addr")]
    pub containerd_addr: String,
    /// CLI binary name: "nerdctl" (default) or "docker".
    #[serde(default = "default_cli")]
    pub cli: String,
    #[serde(default)]
    pub tls: TlsConfig,
    #[serde(default)]
    pub registry_auth: HashMap<String, RegistryAuth>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeMountConfig {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    pub ca: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryAuth {
    pub username: String,
    pub password: String,
}

fn default_user() -> String {
    "65534:65534".to_string()
}

fn default_network() -> String {
    "none".to_string()
}

fn default_pull() -> String {
    "if-not-present".to_string()
}

fn default_containerd_addr() -> String {
    "/run/containerd/containerd.sock".to_string()
}

fn default_cli() -> String {
    "nerdctl".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn serde_round_trip_full_config() {
        let config = ContainerdConfig {
            image: "alpine:3.18".to_string(),
            command: Some(vec!["echo".to_string(), "hello".to_string()]),
            run: None,
            env: HashMap::from([("FOO".to_string(), "bar".to_string())]),
            volumes: vec![VolumeMountConfig {
                source: "/host/path".to_string(),
                target: "/container/path".to_string(),
                readonly: true,
            }],
            working_dir: Some("/app".to_string()),
            user: "1000:1000".to_string(),
            network: "host".to_string(),
            memory: Some("512m".to_string()),
            cpu: Some("1.0".to_string()),
            pull: "always".to_string(),
            containerd_addr: "/custom/containerd.sock".to_string(),
            cli: "nerdctl".to_string(),
            tls: TlsConfig {
                ca: Some("/ca.pem".to_string()),
                cert: Some("/cert.pem".to_string()),
                key: Some("/key.pem".to_string()),
            },
            registry_auth: HashMap::from([(
                "registry.example.com".to_string(),
                RegistryAuth {
                    username: "user".to_string(),
                    password: "pass".to_string(),
                },
            )]),
            timeout_ms: Some(30000),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ContainerdConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.image, config.image);
        assert_eq!(deserialized.command, config.command);
        assert_eq!(deserialized.run, config.run);
        assert_eq!(deserialized.env, config.env);
        assert_eq!(deserialized.volumes.len(), 1);
        assert_eq!(deserialized.volumes[0].source, "/host/path");
        assert_eq!(deserialized.volumes[0].readonly, true);
        assert_eq!(deserialized.working_dir, Some("/app".to_string()));
        assert_eq!(deserialized.user, "1000:1000");
        assert_eq!(deserialized.network, "host");
        assert_eq!(deserialized.memory, Some("512m".to_string()));
        assert_eq!(deserialized.cpu, Some("1.0".to_string()));
        assert_eq!(deserialized.pull, "always");
        assert_eq!(deserialized.containerd_addr, "/custom/containerd.sock");
        assert_eq!(deserialized.tls.ca, Some("/ca.pem".to_string()));
        assert_eq!(deserialized.tls.cert, Some("/cert.pem".to_string()));
        assert_eq!(deserialized.tls.key, Some("/key.pem".to_string()));
        assert!(
            deserialized
                .registry_auth
                .contains_key("registry.example.com")
        );
        assert_eq!(deserialized.timeout_ms, Some(30000));
    }

    #[test]
    fn serde_round_trip_minimal_config() {
        let json = r#"{"image": "alpine:latest"}"#;
        let config: ContainerdConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.image, "alpine:latest");
        assert_eq!(config.command, None);
        assert_eq!(config.run, None);
        assert!(config.env.is_empty());
        assert!(config.volumes.is_empty());
        assert_eq!(config.working_dir, None);
        assert_eq!(config.user, "65534:65534");
        assert_eq!(config.network, "none");
        assert_eq!(config.memory, None);
        assert_eq!(config.cpu, None);
        assert_eq!(config.pull, "if-not-present");
        assert_eq!(config.containerd_addr, "/run/containerd/containerd.sock");
        assert_eq!(config.timeout_ms, None);

        // Round-trip
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: ContainerdConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.image, "alpine:latest");
        assert_eq!(deserialized.user, "65534:65534");
    }

    #[test]
    fn default_values() {
        let json = r#"{"image": "busybox"}"#;
        let config: ContainerdConfig = serde_json::from_str(json).unwrap();

        assert_eq!(config.user, "65534:65534");
        assert_eq!(config.network, "none");
        assert_eq!(config.pull, "if-not-present");
        assert_eq!(config.containerd_addr, "/run/containerd/containerd.sock");
    }

    #[test]
    fn volume_mount_serde() {
        let vol = VolumeMountConfig {
            source: "/data".to_string(),
            target: "/mnt/data".to_string(),
            readonly: false,
        };
        let json = serde_json::to_string(&vol).unwrap();
        let deserialized: VolumeMountConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.source, "/data");
        assert_eq!(deserialized.target, "/mnt/data");
        assert_eq!(deserialized.readonly, false);

        // With readonly=true
        let vol_ro = VolumeMountConfig {
            source: "/src".to_string(),
            target: "/dest".to_string(),
            readonly: true,
        };
        let json_ro = serde_json::to_string(&vol_ro).unwrap();
        let deserialized_ro: VolumeMountConfig = serde_json::from_str(&json_ro).unwrap();
        assert_eq!(deserialized_ro.readonly, true);
    }

    #[test]
    fn tls_config_defaults() {
        let tls = TlsConfig::default();
        assert_eq!(tls.ca, None);
        assert_eq!(tls.cert, None);
        assert_eq!(tls.key, None);

        let json = r#"{}"#;
        let deserialized: TlsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.ca, None);
        assert_eq!(deserialized.cert, None);
        assert_eq!(deserialized.key, None);
    }

    #[test]
    fn registry_auth_serde() {
        let auth = RegistryAuth {
            username: "admin".to_string(),
            password: "secret123".to_string(),
        };
        let json = serde_json::to_string(&auth).unwrap();
        let deserialized: RegistryAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.username, "admin");
        assert_eq!(deserialized.password, "secret123");
    }
}
