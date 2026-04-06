use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use wfe_core::models::service::{ServiceDefinition, ServiceEndpoint};
use wfe_core::traits::ServiceProvider;

/// Provisions infrastructure services as containerd containers on the host network.
///
/// Services are accessible via `127.0.0.1` on their declared ports.
/// Connection info is injected as `SVC_{NAME}_HOST` / `SVC_{NAME}_PORT` env vars
/// into workflow data.
pub struct ContainerdServiceProvider {
    containerd_addr: String,
    /// Track running service containers per workflow for teardown.
    running: Mutex<HashMap<String, Vec<String>>>,
}

impl ContainerdServiceProvider {
    pub fn new(containerd_addr: impl Into<String>) -> Self {
        Self {
            containerd_addr: containerd_addr.into(),
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Get the containerd address this provider connects to.
    pub fn containerd_addr(&self) -> &str {
        &self.containerd_addr
    }
}

#[async_trait]
impl ServiceProvider for ContainerdServiceProvider {
    fn can_provision(&self, _services: &[ServiceDefinition]) -> bool {
        true // containerd can run any OCI image
    }

    async fn provision(
        &self,
        workflow_id: &str,
        services: &[ServiceDefinition],
    ) -> wfe_core::Result<Vec<ServiceEndpoint>> {
        let mut endpoints = Vec::new();
        let mut container_ids = Vec::new();

        for svc in services {
            let container_id = format!("wfe-svc-{}-{}", svc.name, workflow_id);

            // Create and start the service container via containerd gRPC.
            // This reuses the same connection and container lifecycle as ContainerdStep
            // but starts the container without waiting for it to exit.
            crate::step::ContainerdStep::run_service(
                &self.containerd_addr,
                &container_id,
                &svc.image,
                &svc.env,
            )
            .await?;

            container_ids.push(container_id);

            endpoints.push(ServiceEndpoint {
                name: svc.name.clone(),
                host: "127.0.0.1".into(),
                ports: svc.ports.clone(),
            });
        }

        self.running
            .lock()
            .unwrap()
            .insert(workflow_id.into(), container_ids);

        Ok(endpoints)
    }

    async fn teardown(&self, workflow_id: &str) -> wfe_core::Result<()> {
        let ids = self
            .running
            .lock()
            .unwrap()
            .remove(workflow_id)
            .unwrap_or_default();

        for container_id in ids {
            crate::step::ContainerdStep::cleanup_service(&self.containerd_addr, &container_id)
                .await
                .ok();
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wfe_core::models::service::ServicePort;

    #[test]
    fn can_provision_always_true() {
        let provider = ContainerdServiceProvider::new("/run/containerd/containerd.sock");
        let services = vec![ServiceDefinition {
            name: "postgres".into(),
            image: "postgres:15".into(),
            ports: vec![ServicePort::tcp(5432)],
            env: Default::default(),
            readiness: None,
            command: vec![],
            args: vec![],
            memory: None,
            cpu: None,
        }];
        assert!(provider.can_provision(&services));
    }

    #[test]
    fn can_provision_empty_services() {
        let provider = ContainerdServiceProvider::new("/run/containerd/containerd.sock");
        assert!(provider.can_provision(&[]));
    }

    #[test]
    fn running_map_starts_empty() {
        let provider = ContainerdServiceProvider::new("/run/containerd/containerd.sock");
        assert!(provider.running.lock().unwrap().is_empty());
    }

    #[test]
    fn containerd_addr_accessor() {
        let provider = ContainerdServiceProvider::new("http://127.0.0.1:2500");
        assert_eq!(provider.containerd_addr(), "http://127.0.0.1:2500");
    }
}
