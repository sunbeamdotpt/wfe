use std::time::Duration;

use wfe_core::models::{WorkflowInstance, WorkflowStatus};
use wfe_core::{Result, WfeError};

use crate::host::WorkflowHost;

/// Run a workflow to completion synchronously (for testing).
///
/// Starts the workflow, then polls persistence in a loop until the workflow
/// reaches `Complete` or `Terminated` status, or the timeout expires.
pub async fn run_workflow_sync(
    host: &WorkflowHost,
    definition_id: &str,
    version: u32,
    data: serde_json::Value,
    timeout: Duration,
) -> Result<WorkflowInstance> {
    let workflow_id = host.start_workflow(definition_id, version, data).await?;

    let start = tokio::time::Instant::now();
    let poll_interval = Duration::from_millis(25);

    loop {
        if start.elapsed() > timeout {
            return Err(WfeError::StepExecution(format!(
                "Workflow {workflow_id} did not complete within {timeout:?}"
            )));
        }

        let instance = host.persistence.get_workflow_instance(&workflow_id).await?;

        match instance.status {
            WorkflowStatus::Complete | WorkflowStatus::Terminated => {
                return Ok(instance);
            }
            _ => {
                tokio::time::sleep(poll_interval).await;
            }
        }
    }
}
