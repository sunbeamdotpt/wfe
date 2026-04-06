use kube::Client;
use wfe_core::WfeError;

use crate::config::ClusterConfig;

/// Create a Kubernetes API client from the cluster configuration.
///
/// Resolution order:
/// 1. If `kubeconfig` path is set, load from that file.
/// 2. Try in-cluster config (for pods running inside K8s).
/// 3. Fall back to default kubeconfig (~/.kube/config).
pub async fn create_client(config: &ClusterConfig) -> Result<Client, WfeError> {
    let kube_config = if let Some(ref path) = config.kubeconfig {
        kube::Config::from_custom_kubeconfig(
            kube::config::Kubeconfig::read_from(path).map_err(|e| {
                WfeError::StepExecution(format!("failed to read kubeconfig at '{path}': {e}"))
            })?,
            &kube::config::KubeConfigOptions::default(),
        )
        .await
        .map_err(|e| {
            WfeError::StepExecution(format!(
                "failed to build config from kubeconfig '{path}': {e}"
            ))
        })?
    } else {
        kube::Config::infer()
            .await
            .map_err(|e| WfeError::StepExecution(format!("failed to infer K8s config: {e}")))?
    };

    Client::try_from(kube_config)
        .map_err(|e| WfeError::StepExecution(format!("failed to create K8s client: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_client_with_bad_kubeconfig_path() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let config = ClusterConfig {
            kubeconfig: Some("/nonexistent/kubeconfig".into()),
            ..Default::default()
        };
        let result = rt.block_on(create_client(&config));
        let err = match result {
            Err(e) => format!("{e}"),
            Ok(_) => panic!("expected error for nonexistent kubeconfig"),
        };
        assert!(err.contains("kubeconfig"));
    }
}
