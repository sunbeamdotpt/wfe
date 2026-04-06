use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Cluster-level configuration shared across all Kubernetes steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Path to kubeconfig file. None uses in-cluster config or default ~/.kube/config.
    #[serde(default)]
    pub kubeconfig: Option<String>,
    /// Namespace prefix for auto-generated namespaces. Default: "wfe-".
    #[serde(default = "default_namespace_prefix")]
    pub namespace_prefix: String,
    /// ServiceAccount name for Job pods.
    #[serde(default)]
    pub service_account: Option<String>,
    /// Image pull secret names.
    #[serde(default)]
    pub image_pull_secrets: Vec<String>,
    /// Node selector labels for Job pods.
    #[serde(default)]
    pub node_selector: HashMap<String, String>,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            kubeconfig: None,
            namespace_prefix: default_namespace_prefix(),
            service_account: None,
            image_pull_secrets: Vec::new(),
            node_selector: HashMap::new(),
        }
    }
}

fn default_namespace_prefix() -> String {
    "wfe-".to_string()
}

/// Per-step configuration for a Kubernetes Job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubernetesStepConfig {
    /// Container image to run.
    pub image: String,
    /// Override entrypoint.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Shorthand: runs via `/bin/sh -c "..."`. Mutually exclusive with `command`.
    #[serde(default)]
    pub run: Option<String>,
    /// Environment variables injected into the container.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory inside the container.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Memory limit (e.g., "512Mi", "1Gi").
    #[serde(default)]
    pub memory: Option<String>,
    /// CPU limit (e.g., "500m", "1").
    #[serde(default)]
    pub cpu: Option<String>,
    /// Execution timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Image pull policy: Always, IfNotPresent, Never.
    #[serde(default)]
    pub pull_policy: Option<String>,
    /// Override the auto-generated namespace.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn cluster_config_defaults() {
        let config = ClusterConfig::default();
        assert_eq!(config.namespace_prefix, "wfe-");
        assert!(config.kubeconfig.is_none());
        assert!(config.service_account.is_none());
        assert!(config.image_pull_secrets.is_empty());
        assert!(config.node_selector.is_empty());
    }

    #[test]
    fn cluster_config_serde_round_trip() {
        let config = ClusterConfig {
            kubeconfig: Some("/home/user/.kube/config".into()),
            namespace_prefix: "test-".into(),
            service_account: Some("wfe-runner".into()),
            image_pull_secrets: vec!["ghcr-secret".into()],
            node_selector: [("tier".into(), "compute".into())].into(),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ClusterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.namespace_prefix, "test-");
        assert_eq!(parsed.service_account, Some("wfe-runner".into()));
        assert_eq!(parsed.image_pull_secrets, vec!["ghcr-secret"]);
    }

    #[test]
    fn step_config_minimal() {
        let json = r#"{"image": "alpine:3.18"}"#;
        let config: KubernetesStepConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.image, "alpine:3.18");
        assert!(config.command.is_none());
        assert!(config.run.is_none());
        assert!(config.env.is_empty());
    }

    #[test]
    fn step_config_full_serde_round_trip() {
        let config = KubernetesStepConfig {
            image: "node:20-alpine".into(),
            command: None,
            run: Some("npm test".into()),
            env: [("NODE_ENV".into(), "test".into())].into(),
            working_dir: Some("/app".into()),
            memory: Some("512Mi".into()),
            cpu: Some("500m".into()),
            timeout_ms: Some(300_000),
            pull_policy: Some("IfNotPresent".into()),
            namespace: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: KubernetesStepConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.image, "node:20-alpine");
        assert_eq!(parsed.run, Some("npm test".into()));
        assert_eq!(parsed.env.get("NODE_ENV"), Some(&"test".to_string()));
        assert_eq!(parsed.memory, Some("512Mi".into()));
        assert_eq!(parsed.timeout_ms, Some(300_000));
    }

    #[test]
    fn step_config_with_command() {
        let json = r#"{"image": "gcc:latest", "command": ["make", "build"]}"#;
        let config: KubernetesStepConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.command, Some(vec!["make".into(), "build".into()]));
        assert!(config.run.is_none());
    }
}
