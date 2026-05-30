use std::collections::{BTreeMap, HashMap};

use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, LocalObjectReference, PersistentVolumeClaimVolumeSource, PodSpec,
    PodTemplateSpec, ResourceRequirements, Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::ObjectMeta;

use crate::config::{ClusterConfig, KubernetesStepConfig};

const LABEL_STEP_NAME: &str = "wfe.sunbeam.pt/step-name";
const LABEL_MANAGED_BY: &str = "wfe.sunbeam.pt/managed-by";
/// Name of the pod volume referencing the shared PVC. Stable so VolumeMount
/// name and Volume name stay in sync inside a single pod spec.
const SHARED_VOLUME_NAME: &str = "wfe-workspace";

/// Request that the generated Job mount a pre-existing PVC into every
/// step container at `mount_path`. Passed in by the step executor, which
/// is responsible for creating the PVC before calling `build_job`.
#[derive(Debug, Clone)]
pub struct SharedVolumeMount {
    /// Claim name.
    pub claim_name: String,
    /// Mount path.
    pub mount_path: String,
}

/// Build a Kubernetes Job manifest from step configuration.
pub fn build_job(
    config: &KubernetesStepConfig,
    step_name: &str,
    namespace: &str,
    env_overrides: &HashMap<String, String>,
    cluster: &ClusterConfig,
    shared_volume: Option<&SharedVolumeMount>,
    has_artifacts: bool,
    artifact_input_names: &[String],
) -> Job {
    let job_name = sanitize_name(step_name);

    // Build container command.
    let (command, mut args) = resolve_command(config);

    // When there are artifact outputs, append a short sleep to give the
    // executor time to copy files out before the pod completes.
    if !config.artifact_outputs.is_empty() {
        if let Some(ref mut a) = args {
            if let Some(first) = a.first_mut() {
                first.push_str(" && sleep 5");
            }
        }
    }

    // Merge environment variables: overrides first, then config (config wins).
    let env_vars = build_env_vars(env_overrides, &config.env);

    // Resource limits.
    let resources = build_resources(&config.memory, &config.cpu);

    // Image pull policy.
    let pull_policy = config
        .pull_policy
        .clone()
        .unwrap_or_else(|| "IfNotPresent".to_string());

    let mut labels = BTreeMap::new();
    labels.insert(LABEL_STEP_NAME.into(), step_name.to_string());
    labels.insert(LABEL_MANAGED_BY.into(), "wfe-kubernetes".into());

    let mut volume_mounts = Vec::new();
    if let Some(sv) = shared_volume {
        volume_mounts.push(VolumeMount {
            name: SHARED_VOLUME_NAME.into(),
            mount_path: sv.mount_path.clone(),
            ..Default::default()
        });
    }
    if has_artifacts {
        volume_mounts.push(VolumeMount {
            name: "wfe-artifacts".into(),
            mount_path: "/wfe-artifacts".into(),
            ..Default::default()
        });
    }
    let volume_mounts = if volume_mounts.is_empty() {
        None
    } else {
        Some(volume_mounts)
    };

    // Inject INPUT_<NAME> env vars for mounted artifacts.
    let mut env_vars = env_vars;
    for name in artifact_input_names {
        let mount_point = format!("/wfe-artifacts/inputs/{name}");
        env_vars.push(EnvVar {
            name: format!("INPUT_{}", name.to_uppercase()),
            value: Some(mount_point),
            ..Default::default()
        });
    }
    env_vars.sort_by(|a, b| a.name.cmp(&b.name));

    let container = Container {
        name: "step".into(),
        image: Some(config.image.clone()),
        command,
        args,
        env: Some(env_vars),
        working_dir: config.working_dir.clone(),
        resources: Some(resources),
        image_pull_policy: Some(pull_policy),
        volume_mounts,
        ..Default::default()
    };

    let image_pull_secrets = if cluster.image_pull_secrets.is_empty() {
        None
    } else {
        Some(
            cluster
                .image_pull_secrets
                .iter()
                .map(|s| LocalObjectReference { name: s.clone() })
                .collect(),
        )
    };

    let node_selector = if cluster.node_selector.is_empty() {
        None
    } else {
        Some(
            cluster
                .node_selector
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
        )
    };

    let mut volumes = Vec::new();
    if let Some(sv) = shared_volume {
        volumes.push(Volume {
            name: SHARED_VOLUME_NAME.into(),
            persistent_volume_claim: Some(PersistentVolumeClaimVolumeSource {
                claim_name: sv.claim_name.clone(),
                read_only: Some(false),
            }),
            ..Default::default()
        });
    }
    if has_artifacts {
        volumes.push(Volume {
            name: "wfe-artifacts".into(),
            empty_dir: Some(k8s_openapi::api::core::v1::EmptyDirVolumeSource::default()),
            ..Default::default()
        });
    }
    let volumes = if volumes.is_empty() { None } else { Some(volumes) };

    Job {
        metadata: ObjectMeta {
            name: Some(job_name),
            namespace: Some(namespace.to_string()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(0),
            ttl_seconds_after_finished: Some(300),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![container],
                    restart_policy: Some("Never".into()),
                    service_account_name: cluster.service_account.clone(),
                    image_pull_secrets,
                    node_selector,
                    volumes,
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Resolve command and args from config.
fn resolve_command(config: &KubernetesStepConfig) -> (Option<Vec<String>>, Option<Vec<String>>) {
    if let Some(ref cmd) = config.command {
        (Some(cmd.clone()), None)
    } else if let Some(ref run) = config.run {
        // Default to /bin/sh so minimal container images (alpine/busybox,
        // distroless) work unchanged. Scripts that need bash features
        // (pipefail, arrays, process substitution) should set
        // `shell: /bin/bash` explicitly on the step config.
        let shell = config
            .shell
            .clone()
            .unwrap_or_else(|| "/bin/sh".to_string());
        (Some(vec![shell, "-c".into()]), Some(vec![run.clone()]))
    } else {
        (None, None)
    }
}

/// Build environment variables, merging overrides and config.
fn build_env_vars(
    overrides: &HashMap<String, String>,
    config_env: &HashMap<String, String>,
) -> Vec<EnvVar> {
    let mut merged: HashMap<String, String> = overrides.clone();
    // Config env wins over overrides.
    for (k, v) in config_env {
        merged.insert(k.clone(), v.clone());
    }
    let mut vars: Vec<EnvVar> = merged
        .into_iter()
        .map(|(name, value)| EnvVar {
            name,
            value: Some(value),
            ..Default::default()
        })
        .collect();
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    vars
}

/// Build resource requirements from memory and CPU strings.
fn build_resources(memory: &Option<String>, cpu: &Option<String>) -> ResourceRequirements {
    let mut limits = BTreeMap::new();
    let mut requests = BTreeMap::new();

    if let Some(mem) = memory {
        limits.insert("memory".into(), Quantity(mem.clone()));
        requests.insert("memory".into(), Quantity(mem.clone()));
    }
    if let Some(cpu_val) = cpu {
        limits.insert("cpu".into(), Quantity(cpu_val.clone()));
        requests.insert("cpu".into(), Quantity(cpu_val.clone()));
    }

    ResourceRequirements {
        limits: if limits.is_empty() {
            None
        } else {
            Some(limits)
        },
        requests: if requests.is_empty() {
            None
        } else {
            Some(requests)
        },
        ..Default::default()
    }
}

/// Sanitize a step name for use as a K8s resource name.
/// Must be lowercase alphanumeric + hyphens, max 63 chars.
pub fn sanitize_name(name: &str) -> String {
    let sanitized: String = name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    sanitized.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn default_cluster() -> ClusterConfig {
        ClusterConfig::default()
    }

    #[test]
    fn build_job_minimal() {
        let config = KubernetesStepConfig {
            image: "alpine:3.18".into(),
            command: None,
            run: None,
            shell: None,
            env: HashMap::new(),
            working_dir: None,
            memory: None,
            cpu: None,
            timeout_ms: None,
            pull_policy: None,
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let job = build_job(
            &config,
            "test-step",
            "wfe-abc",
            &HashMap::new(),
            &default_cluster(),
            None,
            false,
            &[],
        );

        assert_eq!(job.metadata.name, Some("test-step".into()));
        assert_eq!(job.metadata.namespace, Some("wfe-abc".into()));

        let spec = job.spec.as_ref().unwrap();
        assert_eq!(spec.backoff_limit, Some(0));
        assert_eq!(spec.ttl_seconds_after_finished, Some(300));

        let pod_spec = spec.template.spec.as_ref().unwrap();
        assert_eq!(pod_spec.restart_policy, Some("Never".into()));
        assert_eq!(pod_spec.containers.len(), 1);

        let container = &pod_spec.containers[0];
        assert_eq!(container.image, Some("alpine:3.18".into()));
        assert_eq!(container.image_pull_policy, Some("IfNotPresent".into()));
        assert!(container.command.is_none());
        assert!(container.args.is_none());
    }

    #[test]
    fn build_job_with_run_command() {
        let config = KubernetesStepConfig {
            image: "node:20".into(),
            command: None,
            run: Some("npm test".into()),
            shell: None,
            env: HashMap::new(),
            working_dir: Some("/app".into()),
            memory: None,
            cpu: None,
            timeout_ms: None,
            pull_policy: None,
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let job = build_job(
            &config,
            "test",
            "ns",
            &HashMap::new(),
            &default_cluster(),
            None,
            false,
            &[],
        );
        let container = &job.spec.unwrap().template.spec.unwrap().containers[0];

        assert_eq!(container.command, Some(vec!["/bin/sh".into(), "-c".into()]));
        assert_eq!(container.args, Some(vec!["npm test".into()]));
        assert_eq!(container.working_dir, Some("/app".into()));
    }

    #[test]
    fn build_job_with_explicit_command() {
        let config = KubernetesStepConfig {
            image: "gcc:latest".into(),
            command: Some(vec!["make".into(), "build".into()]),
            run: None,
            shell: None,
            env: HashMap::new(),
            working_dir: None,
            memory: None,
            cpu: None,
            timeout_ms: None,
            pull_policy: None,
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let job = build_job(
            &config,
            "build",
            "ns",
            &HashMap::new(),
            &default_cluster(),
            None,
            false,
            &[],
        );
        let container = &job.spec.unwrap().template.spec.unwrap().containers[0];

        assert_eq!(container.command, Some(vec!["make".into(), "build".into()]));
        assert!(container.args.is_none());
    }

    #[test]
    fn build_job_merges_env_vars() {
        let config = KubernetesStepConfig {
            image: "alpine".into(),
            command: None,
            run: None,
            shell: None,
            env: [("APP_ENV".into(), "production".into())].into(),
            working_dir: None,
            memory: None,
            cpu: None,
            timeout_ms: None,
            pull_policy: None,
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let overrides: HashMap<String, String> = [
            ("WORKFLOW_ID".into(), "wf-123".into()),
            ("APP_ENV".into(), "staging".into()),
        ]
        .into();

        let job = build_job(&config, "step", "ns", &overrides, &default_cluster(), None, false, &[]);
        let container = &job.spec.unwrap().template.spec.unwrap().containers[0];
        let env = container.env.as_ref().unwrap();

        // Config env wins over overrides.
        let app_env = env.iter().find(|e| e.name == "APP_ENV").unwrap();
        assert_eq!(app_env.value, Some("production".into()));

        // Override still present.
        let wf_id = env.iter().find(|e| e.name == "WORKFLOW_ID").unwrap();
        assert_eq!(wf_id.value, Some("wf-123".into()));
    }

    #[test]
    fn build_job_with_resources() {
        let config = KubernetesStepConfig {
            image: "alpine".into(),
            command: None,
            run: None,
            shell: None,
            env: HashMap::new(),
            working_dir: None,
            memory: Some("512Mi".into()),
            cpu: Some("500m".into()),
            timeout_ms: None,
            pull_policy: None,
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let job = build_job(
            &config,
            "step",
            "ns",
            &HashMap::new(),
            &default_cluster(),
            None,
            false,
            &[],
        );
        let container = &job.spec.unwrap().template.spec.unwrap().containers[0];
        let resources = container.resources.as_ref().unwrap();

        let limits = resources.limits.as_ref().unwrap();
        assert_eq!(limits.get("memory"), Some(&Quantity("512Mi".into())));
        assert_eq!(limits.get("cpu"), Some(&Quantity("500m".into())));

        let requests = resources.requests.as_ref().unwrap();
        assert_eq!(requests.get("memory"), Some(&Quantity("512Mi".into())));
    }

    #[test]
    fn build_job_with_cluster_config() {
        let cluster = ClusterConfig {
            service_account: Some("wfe-runner".into()),
            image_pull_secrets: vec!["ghcr-secret".into()],
            node_selector: [("tier".into(), "compute".into())].into(),
            ..Default::default()
        };
        let config = KubernetesStepConfig {
            image: "alpine".into(),
            command: None,
            run: None,
            shell: None,
            env: HashMap::new(),
            working_dir: None,
            memory: None,
            cpu: None,
            timeout_ms: None,
            pull_policy: Some("Always".into()),
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let job = build_job(&config, "step", "ns", &HashMap::new(), &cluster, None, false, &[]);
        let pod_spec = job.spec.unwrap().template.spec.unwrap();

        assert_eq!(pod_spec.service_account_name, Some("wfe-runner".into()));
        assert_eq!(pod_spec.image_pull_secrets.unwrap().len(), 1);
        assert_eq!(
            pod_spec.node_selector.unwrap().get("tier"),
            Some(&"compute".to_string())
        );

        let container = &pod_spec.containers[0];
        assert_eq!(container.image_pull_policy, Some("Always".into()));
    }

    #[test]
    fn build_job_labels() {
        let config = KubernetesStepConfig {
            image: "alpine".into(),
            command: None,
            run: None,
            shell: None,
            env: HashMap::new(),
            working_dir: None,
            memory: None,
            cpu: None,
            timeout_ms: None,
            pull_policy: None,
            namespace: None,
            artifact_outputs: HashMap::new(),
        };
        let job = build_job(
            &config,
            "my-step",
            "ns",
            &HashMap::new(),
            &default_cluster(),
            None,
            false,
            &[],
        );
        let labels = job.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get(LABEL_STEP_NAME), Some(&"my-step".to_string()));
        assert_eq!(
            labels.get(LABEL_MANAGED_BY),
            Some(&"wfe-kubernetes".to_string())
        );
    }

    #[test]
    fn sanitize_name_basic() {
        assert_eq!(sanitize_name("my-step"), "my-step");
    }

    #[test]
    fn sanitize_name_special_chars() {
        assert_eq!(sanitize_name("my_step.v1"), "my-step-v1");
    }

    #[test]
    fn sanitize_name_uppercase() {
        assert_eq!(sanitize_name("MyStep"), "mystep");
    }

    #[test]
    fn sanitize_name_truncates() {
        let long = "a".repeat(100);
        assert!(sanitize_name(&long).len() <= 63);
    }

    #[test]
    fn sanitize_name_trims_trailing_hyphens() {
        assert_eq!(sanitize_name("step---"), "step");
    }

    #[test]
    fn env_vars_sorted_by_name() {
        let overrides: HashMap<String, String> =
            [("ZZZ".into(), "1".into()), ("AAA".into(), "2".into())].into();
        let vars = build_env_vars(&overrides, &HashMap::new());
        assert_eq!(vars[0].name, "AAA");
        assert_eq!(vars[1].name, "ZZZ");
    }
}
