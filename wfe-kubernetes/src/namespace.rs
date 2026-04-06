use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Namespace;
use kube::api::{ObjectMeta, PostParams};
use kube::{Api, Client};
use wfe_core::WfeError;

const LABEL_WORKFLOW_ID: &str = "wfe.sunbeam.pt/workflow-id";
const LABEL_MANAGED_BY: &str = "wfe.sunbeam.pt/managed-by";
const MANAGED_BY_VALUE: &str = "wfe-kubernetes";

/// Generate a namespace name from prefix + workflow ID, truncated to 63 chars.
pub fn namespace_name(prefix: &str, workflow_id: &str) -> String {
    let raw = format!("{prefix}{workflow_id}");
    // K8s namespace names: max 63 chars, lowercase alphanumeric + hyphens
    let sanitized: String = raw
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .take(63)
        .collect();
    // Trim trailing hyphens
    sanitized.trim_end_matches('-').to_string()
}

/// Create a namespace if it doesn't already exist.
pub async fn ensure_namespace(
    client: &Client,
    name: &str,
    workflow_id: &str,
) -> Result<(), WfeError> {
    let api: Api<Namespace> = Api::all(client.clone());

    // Check if it already exists.
    match api.get(name).await {
        Ok(_) => return Ok(()),
        Err(kube::Error::Api(err)) if err.code == 404 => {}
        Err(e) => {
            return Err(WfeError::StepExecution(format!(
                "failed to check namespace '{name}': {e}"
            )));
        }
    }

    let mut labels = BTreeMap::new();
    labels.insert(LABEL_WORKFLOW_ID.into(), workflow_id.to_string());
    labels.insert(LABEL_MANAGED_BY.into(), MANAGED_BY_VALUE.into());

    let ns = Namespace {
        metadata: ObjectMeta {
            name: Some(name.to_string()),
            labels: Some(labels),
            ..Default::default()
        },
        ..Default::default()
    };

    api.create(&PostParams::default(), &ns)
        .await
        .map_err(|e| WfeError::StepExecution(format!("failed to create namespace '{name}': {e}")))?;

    Ok(())
}

/// Delete a namespace and all resources within it.
pub async fn delete_namespace(client: &Client, name: &str) -> Result<(), WfeError> {
    let api: Api<Namespace> = Api::all(client.clone());
    api.delete(name, &Default::default())
        .await
        .map_err(|e| WfeError::StepExecution(format!("failed to delete namespace '{name}': {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn namespace_name_simple() {
        assert_eq!(namespace_name("wfe-", "abc123"), "wfe-abc123");
    }

    #[test]
    fn namespace_name_truncates_to_63() {
        let long_id = "a".repeat(100);
        let name = namespace_name("wfe-", &long_id);
        assert!(name.len() <= 63);
    }

    #[test]
    fn namespace_name_sanitizes_special_chars() {
        assert_eq!(
            namespace_name("wfe-", "my_workflow.v1"),
            "wfe-my-workflow-v1"
        );
    }

    #[test]
    fn namespace_name_lowercases() {
        assert_eq!(namespace_name("WFE-", "MyWorkflow"), "wfe-myworkflow");
    }

    #[test]
    fn namespace_name_trims_trailing_hyphens() {
        let id = "a".repeat(59) + "____";
        let name = namespace_name("wfe-", &id);
        assert!(!name.ends_with('-'));
    }

    #[test]
    fn namespace_name_empty_id() {
        // Trailing hyphen is trimmed.
        assert_eq!(namespace_name("wfe-", ""), "wfe");
    }
}
