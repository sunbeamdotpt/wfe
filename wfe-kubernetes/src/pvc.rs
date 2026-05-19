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
use k8s_openapi::api::storage::v1::StorageClass;
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
        Ok(_) => {}
        // Another step created it between our get and create — also fine.
        Err(kube::Error::Api(err)) if err.code == 409 => {}
        Err(e) => {
            return Err(WfeError::StepExecution(format!(
                "failed to create shared-volume PVC '{name}' in '{namespace}': {e}"
            )));
        }
    }

    // Wait for the PVC to be bound before returning. Storage provisioners
    // (e.g. Longhorn) need a few seconds to create and attach the volume.
    // If we return immediately the Job's pod is created while the PVC is
    // still Pending, and the scheduler rejects it with "unbound immediate
    // PersistentVolumeClaims".
    //
    // However, StorageClasses with `volumeBindingMode: WaitForFirstConsumer`
    // (e.g. rancher.io/local-path) intentionally delay binding until a pod
    // is scheduled. For those, we skip the wait — the scheduler will bind
    // the PVC when the Job's pod is created.
    let sc_name = storage_class.map(|s| s.to_string());
    let sc_name = if sc_name.is_some() {
        sc_name
    } else {
        let sc_api: Api<StorageClass> = Api::all(client.clone());
        match sc_api.list(&Default::default()).await {
            Ok(list) => list.items.into_iter().find(|sc| {
                sc.metadata
                    .annotations
                    .as_ref()
                    .map(|a| a.get("storageclass.kubernetes.io/is-default-class") == Some(&"true".to_string()))
                    .unwrap_or(false)
            }).and_then(|sc| sc.metadata.name),
            Err(_) => None,
        }
    };

    let wait_for_first_consumer = if let Some(sc_name) = sc_name {
        let sc_api: Api<StorageClass> = Api::all(client.clone());
        sc_api
            .get(&sc_name)
            .await
            .ok()
            .and_then(|sc| sc.volume_binding_mode)
            .map(|mode| mode == "WaitForFirstConsumer")
            .unwrap_or(false)
    } else {
        false
    };

    if wait_for_first_consumer {
        return Ok(());
    }

    for _ in 0..60 {
        if let Ok(pvc) = api.get(name).await {
            if let Some(status) = &pvc.status {
                if status.phase.as_deref() == Some("Bound") {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    Err(WfeError::StepExecution(format!(
        "shared-volume PVC '{name}' in '{namespace}' was not bound within 120s"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_volume_pvc_name_is_stable() {
        assert_eq!(shared_volume_pvc_name(), "wfe-workspace");
    }
}
