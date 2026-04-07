//! `wfectl suspend <workflow-id>` -- pause a running workflow.

use anyhow::Result;
use clap::Args;
use wfe_server_protos::wfe::v1::SuspendWorkflowRequest;

use crate::client::AuthClient;

#[derive(Debug, Args)]
pub struct SuspendArgs {
    /// Workflow instance identifier — UUID or human-friendly name (e.g. "ci-42").
    pub workflow_id: String,
}

pub async fn run(args: SuspendArgs, mut client: AuthClient) -> Result<()> {
    client
        .suspend_workflow(SuspendWorkflowRequest {
            workflow_id: args.workflow_id.clone(),
        })
        .await?;
    println!("✓ Suspended workflow {}", args.workflow_id);
    Ok(())
}
