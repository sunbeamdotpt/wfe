use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{ListParams, PostParams};
use kube::{Api, Client};
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};
use wfe_core::WfeError;

use crate::cleanup::delete_job;
use crate::config::{ClusterConfig, KubernetesStepConfig};
use crate::logs::{stream_logs, wait_for_pod_running};
use crate::manifests::build_job;
use crate::namespace::{ensure_namespace, namespace_name};
use crate::output::{build_output_data, parse_outputs};

/// A workflow step that runs as a Kubernetes Job.
pub struct KubernetesStep {
    config: KubernetesStepConfig,
    cluster: ClusterConfig,
    client: Option<Client>,
}

impl KubernetesStep {
    /// Create a step with a pre-built client.
    pub fn new(config: KubernetesStepConfig, cluster: ClusterConfig, client: Client) -> Self {
        Self {
            config,
            cluster,
            client: Some(client),
        }
    }

    /// Create a step that will lazily connect to the cluster on first run.
    pub fn lazy(config: KubernetesStepConfig, cluster: ClusterConfig) -> Self {
        Self {
            config,
            cluster,
            client: None,
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

        let workflow_id = &context.workflow.id;
        let definition_id = &context.workflow.workflow_definition_id;

        // 0. Ensure client is connected.
        let _ = self.get_client().await?;
        let client = self.client.as_ref().unwrap().clone();

        // 1. Determine namespace.
        let ns = self
            .config
            .namespace
            .clone()
            .unwrap_or_else(|| namespace_name(&self.cluster.namespace_prefix, workflow_id));

        // 2. Ensure namespace exists.
        ensure_namespace(&client, &ns, workflow_id).await?;

        // 3. Merge env vars: workflow.data (uppercased) + config.env.
        let env_overrides = extract_workflow_env(&context.workflow.data);

        // 4. Build Job manifest.
        let job_manifest = build_job(
            &self.config,
            &step_name,
            &ns,
            &env_overrides,
            &self.cluster,
        );
        let job_name = job_manifest
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| step_name.clone());

        // 5. Create Job.
        let jobs: Api<Job> = Api::namespaced(client.clone(), &ns);
        jobs.create(&PostParams::default(), &job_manifest)
            .await
            .map_err(|e| {
                WfeError::StepExecution(format!("failed to create job '{job_name}': {e}"))
            })?;

        // Wrap remaining steps in timeout if configured.
        let result = if let Some(timeout_ms) = self.config.timeout_ms {
            match tokio::time::timeout(
                Duration::from_millis(timeout_ms),
                self.execute_job(&client, &ns, &job_name, &step_name, definition_id, workflow_id, context),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    // Timeout — clean up and fail.
                    delete_job(&client, &ns, &job_name).await.ok();
                    return Err(WfeError::StepExecution(format!(
                        "kubernetes job '{job_name}' timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            self.execute_job(&client, &ns, &job_name, &step_name, definition_id, workflow_id, context)
                .await
        };

        // Always attempt cleanup.
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
        // 6. Find the pod created by the Job.
        let pod_name = wait_for_job_pod(client, namespace, job_name).await?;

        // 7. Wait for the pod container to start.
        wait_for_pod_running(client, namespace, &pod_name).await?;

        // 8. Stream logs + capture stdout.
        let stdout = stream_logs(
            client,
            namespace,
            &pod_name,
            step_name,
            definition_id,
            workflow_id,
            context.step.id,
            context.log_sink,
        )
        .await
        .unwrap_or_default();

        // 9. Wait for Job completion and get exit code.
        let (exit_code, stderr) =
            wait_for_job_completion(client, namespace, job_name, &pod_name).await?;

        // 10. Check exit code.
        if exit_code != 0 {
            return Err(WfeError::StepExecution(format!(
                "kubernetes job '{job_name}' exited with code {exit_code}\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        // 11. Parse outputs from stdout.
        let parsed = parse_outputs(&stdout);
        let output_data = build_output_data(step_name, &stdout, &stderr, exit_code, &parsed);

        Ok(ExecutionResult {
            proceed: true,
            output_data: Some(output_data),
            ..Default::default()
        })
    }
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
                WfeError::StepExecution(format!(
                    "failed to list pods for job '{job_name}': {e}"
                ))
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

/// Wait for a Job to reach a terminal state. Returns (exit_code, stderr).
async fn wait_for_job_completion(
    client: &Client,
    namespace: &str,
    job_name: &str,
    pod_name: &str,
) -> Result<(i32, String), WfeError> {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    // Poll Job status.
    for _ in 0..600 {
        let job = jobs.get(job_name).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to get job '{job_name}': {e}"))
        })?;

        if let Some(status) = &job.status {
            if let Some(conditions) = &status.conditions {
                for cond in conditions {
                    if cond.type_ == "Complete" && cond.status == "True" {
                        let exit_code = get_pod_exit_code(&pods, pod_name).await;
                        let stderr = get_pod_stderr(&pods, pod_name).await;
                        return Ok((exit_code, stderr));
                    }
                    if cond.type_ == "Failed" && cond.status == "True" {
                        let exit_code = get_pod_exit_code(&pods, pod_name).await;
                        let stderr = get_pod_stderr(&pods, pod_name).await;
                        return Ok((exit_code, stderr));
                    }
                }
            }

            // Also check succeeded/failed counts.
            if status.succeeded.unwrap_or(0) > 0 {
                let exit_code = get_pod_exit_code(&pods, pod_name).await;
                let stderr = get_pod_stderr(&pods, pod_name).await;
                return Ok((exit_code, stderr));
            }
            if status.failed.unwrap_or(0) > 0 {
                let exit_code = get_pod_exit_code(&pods, pod_name).await;
                let stderr = get_pod_stderr(&pods, pod_name).await;
                return Ok((exit_code, stderr));
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(WfeError::StepExecution(format!(
        "job '{job_name}' did not complete within 600s"
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
        assert_eq!(
            env.get("CONFIG"),
            Some(&r#"{"nested":true}"#.to_string())
        );
    }
}
