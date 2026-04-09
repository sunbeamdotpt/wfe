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
    /// Default size (e.g. "10Gi") used when a workflow declares a
    /// `shared_volume` without specifying its own `size`. The K8s executor
    /// creates one PVC per top-level workflow instance and mounts it on
    /// every step container so sub-workflows can share a cloned checkout,
    /// sccache directory, etc. Cluster operators tune this based on the
    /// typical working-set size of their pipelines.
    #[serde(default = "default_shared_volume_size")]
    pub default_shared_volume_size: String,
    /// Optional StorageClass to use when provisioning shared-volume PVCs.
    /// Falls back to the cluster's default StorageClass when unset.
    #[serde(default)]
    pub shared_volume_storage_class: Option<String>,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            kubeconfig: None,
            namespace_prefix: default_namespace_prefix(),
            service_account: None,
            image_pull_secrets: Vec::new(),
            node_selector: HashMap::new(),
            default_shared_volume_size: default_shared_volume_size(),
            shared_volume_storage_class: None,
        }
    }
}

fn default_namespace_prefix() -> String {
    "wfe-".to_string()
}

fn default_shared_volume_size() -> String {
    "10Gi".to_string()
}

/// Per-step configuration for a Kubernetes Job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KubernetesStepConfig {
    /// Container image to run.
    pub image: String,
    /// Override entrypoint.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Shorthand: runs the given script via the configured `shell`
    /// (default `/bin/sh`). Mutually exclusive with `command`. For scripts
    /// that rely on bashisms like `set -o pipefail`, process substitution,
    /// or arrays, set `shell: /bin/bash` explicitly — the default /bin/sh
    /// keeps alpine/busybox containers working out of the box.
    #[serde(default)]
    pub run: Option<String>,
    /// Shell used to execute a `run:` script. Defaults to `/bin/sh` so
    /// minimal containers (alpine, distroless) work unchanged. Override
    /// to `/bin/bash` or any other interpreter when the script needs
    /// features dash doesn't support.
    #[serde(default)]
    pub shell: Option<String>,
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
            default_shared_volume_size: "20Gi".into(),
            shared_volume_storage_class: Some("fast-ssd".into()),
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
            shell: None,
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
