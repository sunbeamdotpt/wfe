use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{ListParams, PostParams};
use kube::{Api, Client};
use wfe_core::WfeError;
use wfe_core::models::{ExecutionResult, artifact_ref_value};
use wfe_core::traits::step::{StepBody, StepExecutionContext};

use crate::cleanup::delete_job;
use crate::config::{ClusterConfig, KubernetesStepConfig};
use crate::logs::{stream_logs, wait_for_pod_running};
use crate::manifests::{SharedVolumeMount, build_job};
use crate::namespace::{ensure_namespace, namespace_name};
use crate::output::{build_output_data, parse_outputs};
use crate::pvc::{ensure_shared_volume_pvc, shared_volume_pvc_name};

/// A workflow step that runs as a Kubernetes Job.
pub struct KubernetesStep {
    config: KubernetesStepConfig,
    cluster: ClusterConfig,
    client: Option<Client>,
    /// Temp directory where artifact inputs are extracted.
    artifact_mount_dir: Option<PathBuf>,
    /// Names of artifact inputs that were mounted.
    artifact_input_names: Vec<String>,
}

impl KubernetesStep {
    /// Create a step with a pre-built client.
    pub fn new(config: KubernetesStepConfig, cluster: ClusterConfig, client: Client) -> Self {
        Self {
            config,
            cluster,
            client: Some(client),
            artifact_mount_dir: None,
            artifact_input_names: Vec::new(),
        }
    }

    /// Create a step that will lazily connect to the cluster on first run.
    pub fn lazy(config: KubernetesStepConfig, cluster: ClusterConfig) -> Self {
        Self {
            config,
            cluster,
            client: None,
            artifact_mount_dir: None,
            artifact_input_names: Vec::new(),
        }
    }

    async fn get_client(&mut self) -> wfe_core::Result<&Client> {
        if self.client.is_none() {
            let client = crate::client::create_client(&self.cluster).await?;
            self.client = Some(client);
        }
        Ok(self.client.as_ref().unwrap())
    }
}

#[async_trait]
impl StepBody for KubernetesStep {
    async fn mount_artifacts(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<()> {
        let Some(volume) = context.artifact_volume else {
            return Ok(());
        };

        if volume.is_empty() {
            return Ok(());
        }

        let tmp = std::env::temp_dir().join(format!("wfe-k8s-artifacts-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&tmp).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to create artifact temp dir: {e}"))
        })?;

        // Extract each artifact to <tmp>/inputs/<name>.
        for (name, _) in volume.iter() {
            let dest = tmp.join("inputs").join(name);
            tokio::fs::create_dir_all(&dest).await.map_err(|e| {
                WfeError::StepExecution(format!(
                    "failed to create artifact input dir '{name}': {e}"
                ))
            })?;

            volume.extract_one(name, &dest).map_err(|e| {
                WfeError::StepExecution(format!("failed to extract artifact '{name}': {e}"))
            })?;

            self.artifact_input_names.push(name.to_string());
        }

        self.artifact_mount_dir = Some(tmp);
        Ok(())
    }

    async fn unmount_artifacts(
        &mut self,
        _context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<()> {
        if let Some(dir) = self.artifact_mount_dir.take() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
        self.artifact_input_names.clear();
        Ok(())
    }

    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let step_name = context
            .step
            .name
            .as_deref()
            .unwrap_or("unknown")
            .to_string();

        let isolation_id = context
            .workflow
            .root_workflow_id
            .as_deref()
            .unwrap_or(&context.workflow.id);
        let workflow_id = &context.workflow.id;
        let definition_id = &context.workflow.workflow_definition_id;

        let _ = self.get_client().await?;
        let client = self.client.as_ref().unwrap().clone();

        let ns = self
            .config
            .namespace
            .clone()
            .unwrap_or_else(|| namespace_name(&self.cluster.namespace_prefix, isolation_id));

        ensure_namespace(&client, &ns, workflow_id).await?;

        let shared_mount = {
            let sv_from_def = context.definition.and_then(|d| d.shared_volume.as_ref());

            let sv_from_data: Option<wfe_core::models::SharedVolume> = if sv_from_def.is_none() {
                context
                    .workflow
                    .data
                    .get("_wfe_shared_volume")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
            } else {
                None
            };

            let sv = sv_from_def.or(sv_from_data.as_ref());

            if let Some(sv) = sv {
                let size = sv
                    .size
                    .as_deref()
                    .unwrap_or(&self.cluster.default_shared_volume_size);
                ensure_shared_volume_pvc(
                    &client,
                    &ns,
                    size,
                    self.cluster.shared_volume_storage_class.as_deref(),
                )
                .await?;
                Some(SharedVolumeMount {
                    claim_name: shared_volume_pvc_name().to_string(),
                    mount_path: sv.mount_path.clone(),
                })
            } else {
                None
            }
        };

        let env_overrides = extract_workflow_env(&context.workflow.data);

        let has_artifacts = self.artifact_mount_dir.is_some() || !self.config.artifact_outputs.is_empty();
        let job_manifest = build_job(
            &self.config,
            &step_name,
            &ns,
            &env_overrides,
            &self.cluster,
            shared_mount.as_ref(),
            has_artifacts,
            &self.artifact_input_names,
        );
        let job_name = job_manifest
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| step_name.clone());

        let jobs: Api<Job> = Api::namespaced(client.clone(), &ns);
        jobs.create(&PostParams::default(), &job_manifest)
            .await
            .map_err(|e| {
                WfeError::StepExecution(format!("failed to create job '{job_name}': {e}"))
            })?;

        // Copy artifact inputs into the pod once the container is running.
        if let Some(ref mount_dir) = self.artifact_mount_dir {
            if let Ok(pod_name) = wait_for_job_pod(&client, &ns, &job_name).await {
                if let Err(e) = wait_for_pod_running(&client, &ns, &pod_name).await {
                    delete_job(&client, &ns, &job_name).await.ok();
                    return Err(WfeError::StepExecution(format!(
                        "pod never reached running state for artifact copy: {e}"
                    )));
                }
                // Brief pause: wait_for_pod_running returns when the container
                // transitions to running, but kubectl exec may still fail briefly.
                tokio::time::sleep(Duration::from_millis(500)).await;
                let kubeconfig = self.cluster.kubeconfig.clone();
                if let Err(e) = copy_artifacts_into_pod(&ns, &pod_name, mount_dir, kubeconfig.as_deref()).await {
                    delete_job(&client, &ns, &job_name).await.ok();
                    return Err(WfeError::StepExecution(format!(
                        "failed to copy artifacts into pod: {e}"
                    )));
                }
            }
        }

        let result = if let Some(timeout_ms) = self.config.timeout_ms {
            match tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                self.execute_job(
                    &client,
                    &ns,
                    &job_name,
                    &step_name,
                    definition_id,
                    workflow_id,
                    context,
                ),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    delete_job(&client, &ns, &job_name).await.ok();
                    return Err(WfeError::StepExecution(format!(
                        "kubernetes job '{job_name}' timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            self.execute_job(
                &client,
                &ns,
                &job_name,
                &step_name,
                definition_id,
                workflow_id,
                context,
            )
            .await
        };

        delete_job(&client, &ns, &job_name).await.ok();

        result
    }
}

impl KubernetesStep {
    /// Execute a Job after it's been created: wait for pod, stream logs, check exit code.
    async fn execute_job(
        &self,
        client: &Client,
        namespace: &str,
        job_name: &str,
        step_name: &str,
        definition_id: &str,
        workflow_id: &str,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let pod_name = wait_for_job_pod(client, namespace, job_name).await?;

        wait_for_pod_running(client, namespace, &pod_name).await?;

        let log_workflow_id = context
            .workflow
            .root_workflow_id
            .as_deref()
            .unwrap_or(workflow_id);

        // Stream logs and wait for job completion concurrently so that
        // artifact output copying can happen while the container is still
        // running (stream_logs blocks until the container exits).
        let client_clone = client.clone();
        let ns = namespace.to_string();
        let pod = pod_name.clone();
        let step = step_name.to_string();
        let def = definition_id.to_string();
        let wf = log_workflow_id.to_string();
        let step_id = context.step.id;
        let log_sink = context.log_sink; // lifetime issue - this is a reference

        let logs_future = async move {
            stream_logs(
                &client_clone,
                &ns,
                &pod,
                &step,
                &def,
                &wf,
                step_id,
                log_sink,
            )
            .await
            .unwrap_or_default()
        };

        let kubeconfig = self.cluster.kubeconfig.clone();
        let outputs_future = wait_for_job_completion_with_outputs(
            client,
            namespace,
            job_name,
            &pod_name,
            &self.config.artifact_outputs,
            context.artifact_store,
            kubeconfig.as_deref(),
        );

        let (stdout, outputs_result) = tokio::join!(logs_future, outputs_future);

        let (exit_code, stderr, artifact_refs) = outputs_result?;

        if exit_code != 0 {
            return Err(WfeError::StepExecution(format!(
                "kubernetes job '{job_name}' exited with code {exit_code}\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        let parsed = parse_outputs(&stdout);
        let mut output_data = build_output_data(step_name, &stdout, &stderr, exit_code, &parsed);

        // Merge artifact output references into output_data.
        for (name, artifact_ref) in artifact_refs {
            if let Some(obj) = output_data.as_object_mut() {
                obj.insert(name, artifact_ref);
            }
        }

        Ok(ExecutionResult {
            proceed: true,
            output_data: Some(output_data),
            ..Default::default()
        })
    }
}

/// Copy artifact inputs from a local temp dir into a pod using kubectl cp.
async fn copy_artifacts_into_pod(
    namespace: &str,
    pod_name: &str,
    local_dir: &PathBuf,
    kubeconfig: Option<&str>,
) -> wfe_core::Result<()> {
    let inputs_dir = local_dir.join("inputs");
    if !inputs_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(&inputs_dir).map_err(|e| {
        WfeError::StepExecution(format!("failed to read artifact inputs dir: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            WfeError::StepExecution(format!("failed to read artifact input entry: {e}"))
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        let local_path = entry.path();

        // Ensure the parent directory exists in the pod (with retries).
        let mut mkdir_ok = false;
        for attempt in 0..5 {
            let mut mkdir_cmd = tokio::process::Command::new("kubectl");
            if let Some(kc) = kubeconfig {
                mkdir_cmd.arg("--kubeconfig").arg(kc);
            }
            let mkdir_status = mkdir_cmd
                .args([
                    "exec", "-n", namespace, pod_name, "-c", "step", "--",
                    "mkdir", "-p", "/wfe-artifacts/inputs",
                ])
                .status()
                .await;

            if let Ok(status) = mkdir_status && status.success() {
                mkdir_ok = true;
                break;
            }
            if attempt < 4 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        if !mkdir_ok {
            return Err(WfeError::StepExecution(format!(
                "kubectl exec mkdir failed for artifact '{name}' after retries"
            )));
        }

        // Copy the source directory into /wfe-artifacts/inputs/ (trailing slash
        // tells kubectl cp to place the directory inside the destination).
        let mut cp_ok = false;
        for attempt in 0..5 {
            let mut cp_cmd = tokio::process::Command::new("kubectl");
            if let Some(kc) = kubeconfig {
                cp_cmd.arg("--kubeconfig").arg(kc);
            }
            let status = cp_cmd
                .args([
                    "cp",
                    &local_path.to_string_lossy(),
                    &format!("{namespace}/{pod_name}:/wfe-artifacts/inputs/"),
                    "-c",
                    "step",
                ])
                .status()
                .await;

            if let Ok(status) = status && status.success() {
                cp_ok = true;
                break;
            }
            if attempt < 4 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        if !cp_ok {
            return Err(WfeError::StepExecution(format!(
                "kubectl cp failed for artifact '{name}' after retries"
            )));
        }
    }

    Ok(())
}

/// Wait for a Job to reach a terminal state, copying artifact outputs along the way.
/// Returns (exit_code, stderr, artifact_refs).
async fn wait_for_job_completion_with_outputs(
    client: &Client,
    namespace: &str,
    job_name: &str,
    pod_name: &str,
    artifact_outputs: &HashMap<String, String>,
    artifact_store: Option<&dyn wfe_core::traits::ArtifactStore>,
    kubeconfig: Option<&str>,
) -> Result<(i32, String, HashMap<String, serde_json::Value>), WfeError> {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    let mut copied_outputs: HashMap<String, serde_json::Value> = HashMap::new();

    for iteration in 0..3000 {
        // Try to copy artifact outputs if they exist and haven't been copied yet.
        if !artifact_outputs.is_empty() && artifact_store.is_some() {
            for (name, _container_path) in artifact_outputs {
                if copied_outputs.contains_key(name) {
                    continue;
                }
                let remote_dir = format!("/wfe-artifacts/outputs/{name}");

                // Check if the output directory exists (with retries inside each iteration).
                let mut check_ok = false;
                for attempt in 0..3 {
                    let mut check_cmd = tokio::process::Command::new("kubectl");
                    if let Some(kc) = kubeconfig {
                        check_cmd.arg("--kubeconfig").arg(kc);
                    }
                    let check_status = check_cmd
                        .args([
                            "exec", "-n", namespace, pod_name, "-c", "step", "--",
                            "test", "-d", &remote_dir,
                        ])
                        .status()
                        .await;

                    if let Ok(status) = check_status && status.success() {
                        check_ok = true;
                        break;
                    }
                    if attempt < 2 {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                }

                if check_ok {
                    // Copy output from pod to local temp dir.
                    let output_dir = std::env::temp_dir().join(format!(
                        "wfe-k8s-output-{name}-{}",
                        uuid::Uuid::new_v4()
                    ));
                    tokio::fs::create_dir_all(&output_dir).await.map_err(|e| {
                        WfeError::StepExecution(format!(
                            "failed to create local output dir: {e}"
                        ))
                    })?;

                    let mut cp_ok = false;
                    for attempt in 0..3 {
                        let mut cp_cmd = tokio::process::Command::new("kubectl");
                        if let Some(kc) = kubeconfig {
                            cp_cmd.arg("--kubeconfig").arg(kc);
                        }
                        let cp_status = cp_cmd
                            .args([
                                "cp",
                                &format!("{namespace}/{pod_name}:{remote_dir}"),
                                &output_dir.to_string_lossy(),
                                "-c",
                                "step",
                            ])
                            .status()
                            .await;

                        if let Ok(status) = cp_status && status.success() {
                            cp_ok = true;
                            break;
                        }
                        if attempt < 2 {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }

                    if cp_ok {
                        // Tar + gzip the output directory and store it.
                        let name_for_closure = name.clone();
                        let output_dir_clone = output_dir.clone();
                        let tar_gz = tokio::task::spawn_blocking(move || {
                            let mut buf = Vec::new();
                            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
                            let mut tar = tar::Builder::new(enc);
                            tar.append_dir_all(".", &output_dir_clone).map_err(|e| {
                                WfeError::StepExecution(format!(
                                    "failed to tar artifact output '{name_for_closure}': {e}"
                                ))
                            })?;
                            let enc = tar.into_inner().map_err(|e| {
                                WfeError::StepExecution(format!(
                                    "failed to finish tar for artifact output '{name_for_closure}': {e}"
                                ))
                            })?;
                            enc.finish().map_err(|e| {
                                WfeError::StepExecution(format!(
                                    "failed to finish gzip for artifact output '{name_for_closure}': {e}"
                                ))
                            })?;
                            Ok::<Vec<u8>, WfeError>(buf)
                        })
                        .await
                        .map_err(|e| {
                            WfeError::StepExecution(format!(
                                "artifact output tar task panicked for '{name}': {e}"
                            ))
                        })??;

                        if let Some(store) = artifact_store {
                            let reader: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>> =
                                Box::pin(std::io::Cursor::new(tar_gz));
                            let artifact_ref = store.put(reader).await.map_err(|e| {
                                WfeError::StepExecution(format!(
                                    "failed to store artifact output '{name}': {e}"
                                ))
                            })?;
                            copied_outputs.insert(name.clone(), artifact_ref_value(&artifact_ref.digest));
                        }
                    }
                }
            }
        }

        // Check Job status every 10 iterations (~1 second).
        if iteration % 10 == 0 {
            let job = jobs
                .get(job_name)
                .await
                .map_err(|e| WfeError::StepExecution(format!("failed to get job '{job_name}': {e}")))?;

            if let Some(status) = &job.status {
                if let Some(conditions) = &status.conditions {
                    for cond in conditions {
                        if cond.type_ == "Complete" && cond.status == "True" {
                            let exit_code = get_pod_exit_code(&pods, pod_name).await;
                            let stderr = get_pod_stderr(&pods, pod_name).await;
                            return Ok((exit_code, stderr, copied_outputs));
                        }
                        if cond.type_ == "Failed" && cond.status == "True" {
                            let exit_code = get_pod_exit_code(&pods, pod_name).await;
                            let stderr = get_pod_stderr(&pods, pod_name).await;
                            return Ok((exit_code, stderr, copied_outputs));
                        }
                    }
                }

                if status.succeeded.unwrap_or(0) > 0 {
                    let exit_code = get_pod_exit_code(&pods, pod_name).await;
                    let stderr = get_pod_stderr(&pods, pod_name).await;
                    return Ok((exit_code, stderr, copied_outputs));
                }
                if status.failed.unwrap_or(0) > 0 {
                    let exit_code = get_pod_exit_code(&pods, pod_name).await;
                    let stderr = get_pod_stderr(&pods, pod_name).await;
                    return Ok((exit_code, stderr, copied_outputs));
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(WfeError::StepExecution(format!(
        "job '{job_name}' did not complete within 600s"
    )))
}

/// Wait for the Job to create a pod, returning the pod name.
async fn wait_for_job_pod(
    client: &Client,
    namespace: &str,
    job_name: &str,
) -> Result<String, WfeError> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let selector = format!("job-name={job_name}");

    for _ in 0..60 {
        let pod_list = pods
            .list(&ListParams::default().labels(&selector))
            .await
            .map_err(|e| {
                WfeError::StepExecution(format!("failed to list pods for job '{job_name}': {e}"))
            })?;

        if let Some(pod) = pod_list.items.first() {
            if let Some(name) = &pod.metadata.name {
                return Ok(name.clone());
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(WfeError::StepExecution(format!(
        "no pod created for job '{job_name}' within 60s"
    )))
}

/// Get the exit code from a pod's terminated container.
async fn get_pod_exit_code(pods: &Api<Pod>, pod_name: &str) -> i32 {
    match pods.get(pod_name).await {
        Ok(pod) => pod
            .status
            .and_then(|s| s.container_statuses)
            .and_then(|cs| cs.first().cloned())
            .and_then(|cs| cs.state)
            .and_then(|s| s.terminated)
            .map(|t| t.exit_code)
            .unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Get stderr from a pod's logs (separate container or log endpoint).
/// K8s doesn't separate stdout/stderr in the log API, so we return empty.
async fn get_pod_stderr(_pods: &Api<Pod>, _pod_name: &str) -> String {
    // K8s log API interleaves stdout/stderr. Stderr extraction would require
    // a sidecar pattern. For now, return empty — all output is in stdout.
    String::new()
}

/// Extract workflow data as uppercase environment variables.
pub fn extract_workflow_env(data: &serde_json::Value) -> HashMap<String, String> {
    let mut env = HashMap::new();
    if let Some(obj) = data.as_object() {
        for (key, value) in obj {
            let val_str = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            env.insert(key.to_uppercase(), val_str);
        }
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extract_workflow_env_from_object() {
        let data = serde_json::json!({
            "name": "World",
            "count": 42,
            "flag": true,
        });
        let env = extract_workflow_env(&data);
        assert_eq!(env.get("NAME"), Some(&"World".to_string()));
        assert_eq!(env.get("COUNT"), Some(&"42".to_string()));
        assert_eq!(env.get("FLAG"), Some(&"true".to_string()));
    }

    #[test]
    fn extract_workflow_env_from_non_object() {
        let data = serde_json::json!("just a string");
        let env = extract_workflow_env(&data);
        assert!(env.is_empty());
    }

    #[test]
    fn extract_workflow_env_null() {
        let data = serde_json::Value::Null;
        let env = extract_workflow_env(&data);
        assert!(env.is_empty());
    }

    #[test]
    fn extract_workflow_env_nested_object() {
        let data = serde_json::json!({"config": {"nested": true}});
        let env = extract_workflow_env(&data);
        // Nested object serialized as JSON string.
        assert_eq!(env.get("CONFIG"), Some(&r#"{"nested":true}"#.to_string()));
    }

    #[test]
    fn lazy_constructor_sets_client_to_none() {
        let step = KubernetesStep::lazy(
            KubernetesStepConfig {
                image: "alpine".into(),
                ..Default::default()
            },
            ClusterConfig::default(),
        );
        assert!(step.client.is_none());
        assert!(step.artifact_mount_dir.is_none());
        assert!(step.artifact_input_names.is_empty());
    }

    #[tokio::test]
    async fn mount_artifacts_with_none_volume_returns_ok() {
        let mut step = KubernetesStep::new(
            KubernetesStepConfig {
                image: "alpine".into(),
                ..Default::default()
            },
            ClusterConfig::default(),
            Client::try_default().await.unwrap(),
        );

        let instance = wfe_core::models::WorkflowInstance::new("test", 1, serde_json::json!({}));
        let ws = wfe_core::models::WorkflowStep::new(0, "test-step");
        let pointer = wfe_core::models::ExecutionPointer::new(0);

        let ctx = StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &ws,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
            artifact_store: None,
            artifact_volume: None,
            artifact_package: None,
            persistence: None,
        };

        step.mount_artifacts(&ctx).await.unwrap();
        assert!(step.artifact_mount_dir.is_none());
        assert!(step.artifact_input_names.is_empty());
    }

    #[tokio::test]
    async fn mount_artifacts_with_empty_volume_returns_ok() {
        let mut step = KubernetesStep::new(
            KubernetesStepConfig {
                image: "alpine".into(),
                ..Default::default()
            },
            ClusterConfig::default(),
            Client::try_default().await.unwrap(),
        );

        let instance = wfe_core::models::WorkflowInstance::new("test", 1, serde_json::json!({}));
        let ws = wfe_core::models::WorkflowStep::new(0, "test-step");
        let pointer = wfe_core::models::ExecutionPointer::new(0);
        let volume = wfe_core::ArtifactVolume::from_artifacts(HashMap::new());

        let ctx = StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &ws,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
            artifact_store: None,
            artifact_volume: Some(&volume),
            artifact_package: None,
            persistence: None,
        };

        step.mount_artifacts(&ctx).await.unwrap();
        assert!(step.artifact_mount_dir.is_none());
        assert!(step.artifact_input_names.is_empty());
    }

    #[tokio::test]
    async fn mount_artifacts_extracts_volume_to_temp_dir() {
        let mut step = KubernetesStep::new(
            KubernetesStepConfig {
                image: "alpine".into(),
                ..Default::default()
            },
            ClusterConfig::default(),
            Client::try_default().await.unwrap(),
        );

        // Build a small tar.gz artifact.
        let artifact_bytes = tokio::task::spawn_blocking(|| {
            let mut buf = Vec::new();
            let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut tar = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_path("test.txt").unwrap();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append(&header, std::io::Cursor::new(b"hello")).unwrap();
            let enc = tar.into_inner().unwrap();
            enc.finish().unwrap();
            buf
        })
        .await
        .unwrap();

        let mut artifacts = HashMap::new();
        artifacts.insert("myart".to_string(), bytes::Bytes::from(artifact_bytes));
        let volume = wfe_core::ArtifactVolume::from_artifacts(artifacts);

        let instance = wfe_core::models::WorkflowInstance::new("test", 1, serde_json::json!({}));
        let ws = wfe_core::models::WorkflowStep::new(0, "test-step");
        let pointer = wfe_core::models::ExecutionPointer::new(0);

        let ctx = StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &ws,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
            artifact_store: None,
            artifact_volume: Some(&volume),
            artifact_package: None,
            persistence: None,
        };

        step.mount_artifacts(&ctx).await.unwrap();
        assert!(step.artifact_mount_dir.is_some());
        assert_eq!(step.artifact_input_names, vec!["myart"]);

        // Verify the file was extracted.
        let mount_dir = step.artifact_mount_dir.as_ref().unwrap();
        let extracted = tokio::fs::read_to_string(mount_dir.join("inputs").join("myart").join("test.txt"))
            .await
            .unwrap();
        assert_eq!(extracted, "hello");

        // Clean up.
        step.unmount_artifacts(&ctx).await.unwrap();
        assert!(step.artifact_mount_dir.is_none());
    }

    #[tokio::test]
    async fn unmount_artifacts_without_mount_dir_is_noop() {
        let mut step = KubernetesStep::new(
            KubernetesStepConfig {
                image: "alpine".into(),
                ..Default::default()
            },
            ClusterConfig::default(),
            Client::try_default().await.unwrap(),
        );

        let instance = wfe_core::models::WorkflowInstance::new("test", 1, serde_json::json!({}));
        let ws = wfe_core::models::WorkflowStep::new(0, "test-step");
        let pointer = wfe_core::models::ExecutionPointer::new(0);

        let ctx = StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &ws,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
            artifact_store: None,
            artifact_volume: None,
            artifact_package: None,
            persistence: None,
        };

        step.unmount_artifacts(&ctx).await.unwrap();
        assert!(step.artifact_mount_dir.is_none());
    }

    #[test]
    fn copy_artifacts_into_pod_empty_inputs_dir_returns_ok() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join(format!("wfe-test-empty-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&tmp).await.unwrap();
            // No "inputs" subdirectory — should return Ok immediately.
            let result = copy_artifacts_into_pod("default", "pod-1", &tmp, None).await;
            assert!(result.is_ok());
        });
    }

    #[test]
    fn copy_artifacts_into_pod_mkdir_fails_with_bad_kubeconfig() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join(format!("wfe-test-badkc-{}", uuid::Uuid::new_v4()));
            let inputs = tmp.join("inputs").join("myart");
            tokio::fs::create_dir_all(&inputs).await.unwrap();
            tokio::fs::write(inputs.join("file.txt"), b"data").await.unwrap();

            let bad_kc = "/tmp/nonexistent-kubeconfig-12345.yaml";
            let result = copy_artifacts_into_pod("default", "pod-1", &tmp, Some(bad_kc)).await;
            assert!(result.is_err());
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("mkdir failed"), "expected mkdir failure, got: {err}");
        });
    }

    #[test]
    fn copy_artifacts_into_pod_cp_fails_when_kubectl_returns_error() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt;

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tmp = std::env::temp_dir().join(format!("wfe-test-cpfail-{}", uuid::Uuid::new_v4()));
            let inputs = tmp.join("inputs").join("myart");
            tokio::fs::create_dir_all(&inputs).await.unwrap();
            tokio::fs::write(inputs.join("file.txt"), b"data").await.unwrap();

            // Create a fake kubectl that succeeds for "exec" but fails for "cp",
            // so mkdir passes but cp fails after retries.
            let fake_bin_dir = std::env::temp_dir().join(format!("fake-kubectl-{}", uuid::Uuid::new_v4()));
            tokio::fs::create_dir_all(&fake_bin_dir).await.unwrap();
            #[cfg(unix)]
            {
                let script = r#"#!/bin/sh
if [ "$1" = "exec" ]; then
    exit 0
else
    exit 1
fi
"#;
                tokio::fs::write(fake_bin_dir.join("kubectl"), script).await.unwrap();
                std::fs::set_permissions(fake_bin_dir.join("kubectl"), std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            #[cfg(windows)]
            {
                let script = r#"@echo off
if "%1"=="exec" exit /b 0
exit /b 1
"#;
                tokio::fs::write(fake_bin_dir.join("kubectl.bat"), script).await.unwrap();
            }

            let original_path = std::env::var("PATH").unwrap_or_default();
            #[cfg(unix)]
            unsafe {
                std::env::set_var("PATH", format!("{}:{}", fake_bin_dir.display(), original_path));
            }
            #[cfg(windows)]
            unsafe {
                std::env::set_var("PATH", format!("{};{}", fake_bin_dir.display(), original_path));
            }

            let result = copy_artifacts_into_pod("default", "pod-1", &tmp, None).await;

            // Restore PATH.
            unsafe {
                std::env::set_var("PATH", original_path);
            }

            assert!(result.is_err());
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("cp failed"), "expected cp failure, got: {err}");
        });
    }

    #[tokio::test]
    async fn mount_artifacts_with_invalid_bytes_fails() {
        let mut step = KubernetesStep::new(
            KubernetesStepConfig {
                image: "alpine".into(),
                ..Default::default()
            },
            ClusterConfig::default(),
            Client::try_default().await.unwrap(),
        );

        let mut artifacts = HashMap::new();
        artifacts.insert("badart".to_string(), bytes::Bytes::from_static(b"not a valid tar.gz"));
        let volume = wfe_core::ArtifactVolume::from_artifacts(artifacts);

        let instance = wfe_core::models::WorkflowInstance::new("test", 1, serde_json::json!({}));
        let ws = wfe_core::models::WorkflowStep::new(0, "test-step");
        let pointer = wfe_core::models::ExecutionPointer::new(0);

        let ctx = StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &ws,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
            artifact_store: None,
            artifact_volume: Some(&volume),
            artifact_package: None,
            persistence: None,
        };

        let result = step.mount_artifacts(&ctx).await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("badart"), "expected artifact name in error, got: {err}");
    }

    #[tokio::test]
    async fn get_pod_exit_code_with_missing_pod_returns_minus_one() {
        let client = Client::try_default().await.unwrap();
        let pods: Api<Pod> = Api::namespaced(client, "default");
        let code = get_pod_exit_code(&pods, "nonexistent-pod-12345").await;
        assert_eq!(code, -1);
    }
}
