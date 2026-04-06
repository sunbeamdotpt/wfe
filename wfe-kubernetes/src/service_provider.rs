use std::time::Duration;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use kube::api::PostParams;
use kube::{Api, Client};
use wfe_core::models::service::{ServiceDefinition, ServiceEndpoint};
use wfe_core::traits::ServiceProvider;
use wfe_core::WfeError;

use crate::config::ClusterConfig;
use crate::logs::wait_for_pod_running;
use crate::namespace::{delete_namespace, ensure_namespace, namespace_name};
use crate::service_manifests::{build_k8s_service, build_service_pod};

/// Provisions infrastructure services as Kubernetes Pods + Services.
///
/// Each workflow gets its own namespace. Services are accessible by name
/// via Kubernetes DNS within the namespace.
pub struct KubernetesServiceProvider {
    client: Client,
    config: ClusterConfig,
}

impl KubernetesServiceProvider {
    pub fn new(client: Client, config: ClusterConfig) -> Self {
        Self { client, config }
    }
}

#[async_trait]
impl ServiceProvider for KubernetesServiceProvider {
    fn can_provision(&self, _services: &[ServiceDefinition]) -> bool {
        true // K8s can run any container image
    }

    async fn provision(
        &self,
        workflow_id: &str,
        services: &[ServiceDefinition],
    ) -> wfe_core::Result<Vec<ServiceEndpoint>> {
        let ns = namespace_name(&self.config.namespace_prefix, workflow_id);
        ensure_namespace(&self.client, &ns, workflow_id).await?;

        let mut endpoints = Vec::new();

        for svc in services {
            // Create Pod.
            let pod_manifest = build_service_pod(svc, &ns);
            let pods: Api<Pod> = Api::namespaced(self.client.clone(), &ns);
            pods.create(&PostParams::default(), &pod_manifest)
                .await
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "failed to create service pod '{}': {e}",
                        svc.name
                    ))
                })?;

            // Create K8s Service for DNS.
            let svc_manifest = build_k8s_service(svc, &ns);
            let svcs: Api<k8s_openapi::api::core::v1::Service> =
                Api::namespaced(self.client.clone(), &ns);
            svcs.create(&PostParams::default(), &svc_manifest)
                .await
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "failed to create k8s service '{}': {e}",
                        svc.name
                    ))
                })?;

            // Wait for pod readiness.
            let timeout = svc
                .readiness
                .as_ref()
                .map(|r| Duration::from_millis(r.timeout_ms))
                .unwrap_or(Duration::from_secs(120));

            match tokio::time::timeout(
                timeout,
                wait_for_pod_running(&self.client, &ns, &svc.name),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    return Err(WfeError::StepExecution(format!(
                        "service '{}' pod failed to start: {e}",
                        svc.name
                    )));
                }
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "service '{}' readiness timed out after {}ms",
                        svc.name,
                        timeout.as_millis()
                    )));
                }
            }

            endpoints.push(ServiceEndpoint {
                name: svc.name.clone(),
                host: svc.name.clone(), // K8s DNS resolves within namespace
                ports: svc.ports.clone(),
            });
        }

        Ok(endpoints)
    }

    async fn teardown(&self, workflow_id: &str) -> wfe_core::Result<()> {
        let ns = namespace_name(&self.config.namespace_prefix, workflow_id);
        delete_namespace(&self.client, &ns).await
    }
}
