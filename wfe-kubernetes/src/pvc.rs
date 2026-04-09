//! PersistentVolumeClaim provisioning for cross-step shared workspaces.
//!
//! When a workflow definition declares a `shared_volume`, the K8s executor
//! materializes it as a single PVC in the workflow's namespace. Every step
//! container mounts that PVC at the declared `mount_path`, so sub-workflows
//! of a top-level run see the same filesystem — a clone in `checkout` is
//! still visible to `cargo fmt --check` in `lint`.
//!
//! The PVC name is derived from the namespace (one PVC per namespace) so
//! multiple steps racing to create it are idempotent: the first `ensure_pvc`
//! wins, and every subsequent call sees the existing claim.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    PersistentVolumeClaim, PersistentVolumeClaimSpec, VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::{ObjectMeta, PostParams};
use kube::{Api, Client};
use wfe_core::WfeError;

const LABEL_MANAGED_BY: &str = "wfe.sunbeam.pt/managed-by";
const MANAGED_BY_VALUE: &str = "wfe-kubernetes";

/// Canonical name for the shared volume PVC within a given namespace. Using
/// a stable name (rather than e.g. a UUID) means every step in the same
/// namespace references the same claim without needing to pass the name
/// through workflow metadata.
pub fn shared_volume_pvc_name() -> &'static str {
    "wfe-workspace"
}

/// Create the shared-volume PVC in `namespace` if it does not already exist.
/// `size` is a K8s resource quantity string (e.g. `"10Gi"`). `storage_class`
/// is optional — when `None` the PVC uses the cluster's default StorageClass.
pub async fn ensure_shared_volume_pvc(
    client: &Client,
    namespace: &str,
    size: &str,
    storage_class: Option<&str>,
) -> Result<(), WfeError> {
    let api: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), namespace);
    let name = shared_volume_pvc_name();

    // Idempotent: if it already exists we're done. This races cleanly with
    // concurrent steps because `get` + `create(AlreadyExists)` is tolerated.
    if api.get(name).await.is_ok() {
        return Ok(());
    }

    let mut labels = BTreeMap::new();
    labels.insert(LABEL_MANAGED_BY.into(), MANAGED_BY_VALUE.into());

    let pvc = PersistentVolumeClaim {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            namespace: Some(namespace.to_string()),
            labels: Some(labels),
            ..Default::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
            resources: Some(VolumeResourceRequirements {
                requests: Some(
                    [("storage".to_string(), Quantity(size.to_string()))]
                        .into_iter()
                        .collect(),
                ),
                ..Default::default()
            }),
            storage_class_name: storage_class.map(|s| s.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    match api.create(&PostParams::default(), &pvc).await {
        Ok(_) => Ok(()),
        // Another step created it between our get and create — also fine.
        Err(kube::Error::Api(err)) if err.code == 409 => Ok(()),
        Err(e) => Err(WfeError::StepExecution(format!(
            "failed to create shared-volume PVC '{name}' in '{namespace}': {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_volume_pvc_name_is_stable() {
        assert_eq!(shared_volume_pvc_name(), "wfe-workspace");
    }
}
