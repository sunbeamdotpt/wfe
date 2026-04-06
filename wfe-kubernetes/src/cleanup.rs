use std::time::Duration;

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::{DeleteParams, ListParams};
use kube::{Api, Client};
use wfe_core::WfeError;

/// Delete a Job by name. The associated Pod is cleaned up via owner references.
pub async fn delete_job(client: &Client, namespace: &str, name: &str) -> Result<(), WfeError> {
    let jobs: Api<Job> = Api::namespaced(client.clone(), namespace);

    let dp = DeleteParams {
        propagation_policy: Some(kube::api::PropagationPolicy::Background),
        ..Default::default()
    };

    match jobs.delete(name, &dp).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(err)) if err.code == 404 => Ok(()),
        Err(e) => Err(WfeError::StepExecution(format!(
            "failed to delete job '{name}' in namespace '{namespace}': {e}"
        ))),
    }
}

/// Clean up stale WFE namespaces older than the given duration.
///
/// Returns the number of namespaces deleted.
pub async fn cleanup_stale_namespaces(
    client: &Client,
    prefix: &str,
    older_than: Duration,
) -> Result<u32, WfeError> {
    let namespaces: Api<Namespace> = Api::all(client.clone());
    let lp = ListParams::default().labels("wfe.sunbeam.pt/managed-by=wfe-kubernetes");

    let ns_list = namespaces
        .list(&lp)
        .await
        .map_err(|e| WfeError::StepExecution(format!("failed to list namespaces: {e}")))?;

    let cutoff_secs = chrono::Utc::now().timestamp() - older_than.as_secs() as i64;
    let mut deleted = 0u32;

    for ns in ns_list {
        let name = ns.metadata.name.as_deref().unwrap_or("");
        if !name.starts_with(prefix) {
            continue;
        }

        // Extract creation timestamp as unix seconds.
        let created_secs = ns
            .metadata
            .creation_timestamp
            .as_ref()
            .map(|t| t.0.as_second())
            .unwrap_or(i64::MAX);

        if created_secs < cutoff_secs {
            if let Err(e) = namespaces.delete(name, &Default::default()).await {
                tracing::warn!("failed to delete stale namespace '{name}': {e}");
            } else {
                tracing::info!("cleaned up stale namespace '{name}'");
                deleted += 1;
            }
        }
    }

    Ok(deleted)
}
