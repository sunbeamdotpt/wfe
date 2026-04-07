//! `wfectl run <definition-id>` -- start a new workflow instance.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use wfe_server_protos::wfe::v1::StartWorkflowRequest;

use crate::client::AuthClient;
use crate::output::OutputFormat;
use crate::struct_util::json_object_to_struct;

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Workflow definition ID.
    pub definition_id: String,
    /// Workflow version (default: 1).
    #[arg(long, default_value_t = 1)]
    pub version: u32,
    /// Path to a JSON file with input data.
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Inline JSON data (overrides --data).
    #[arg(long)]
    pub data_json: Option<String>,
    /// Human-friendly name for this instance. Must be unique across all
    /// workflow instances. Leave unset to let the server auto-assign
    /// `{definition_id}-{N}` using a per-definition monotonic counter.
    #[arg(long)]
    pub name: Option<String>,
}

pub async fn run(args: RunArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
    let data_json = match (args.data_json.as_ref(), args.data.as_ref()) {
        (Some(json), _) => json.clone(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read data file {}", path.display()))?,
        (None, None) => "{}".to_string(),
    };

    let json_value: serde_json::Value =
        serde_json::from_str(&data_json).context("data must be valid JSON")?;
    let data = json_object_to_struct(&json_value);

    let resp = client
        .start_workflow(StartWorkflowRequest {
            definition_id: args.definition_id.clone(),
            version: args.version,
            data: Some(data),
            name: args.name.unwrap_or_default(),
        })
        .await?
        .into_inner();

    if matches!(format, OutputFormat::Json) {
        let json = serde_json::json!({
            "workflow_id": resp.workflow_id,
            "name": resp.name,
            "definition_id": args.definition_id,
            "version": args.version,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("Started workflow: {}", resp.name);
        println!("  ID:         {}", resp.workflow_id);
        println!("  Definition: {} v{}", args.definition_id, args.version);
    }
    Ok(())
}
