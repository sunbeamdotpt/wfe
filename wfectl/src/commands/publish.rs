//! `wfectl publish <event-name> <event-key>` -- publish an event.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use wfe_server_protos::wfe::v1::PublishEventRequest;

use crate::client::AuthClient;
use crate::output::OutputFormat;
use crate::struct_util::json_object_to_struct;

#[derive(Debug, Args)]
pub struct PublishArgs {
    /// Event name (e.g., "order.paid").
    pub event_name: String,
    /// Event key (e.g., the order ID).
    pub event_key: String,
    /// Path to a JSON file with event data.
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Inline JSON data (overrides --data).
    #[arg(long)]
    pub data_json: Option<String>,
}

pub async fn run(args: PublishArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
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
        .publish_event(PublishEventRequest {
            event_name: args.event_name.clone(),
            event_key: args.event_key.clone(),
            data: Some(data),
        })
        .await?
        .into_inner();

    if matches!(format, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "event_id": resp.event_id,
                "event_name": args.event_name,
                "event_key": args.event_key,
            }))?
        );
    } else {
        println!(
            "✓ Published event {} key={} (event_id={})",
            args.event_name, args.event_key, resp.event_id
        );
    }
    Ok(())
}
