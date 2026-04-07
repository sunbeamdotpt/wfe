//! `wfectl logs <workflow-id>` -- stream logs.

use anyhow::Result;
use clap::Args;
use futures::StreamExt;
use wfe_server_protos::wfe::v1::{LogStream, StreamLogsRequest};

use crate::client::AuthClient;

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Workflow instance identifier — UUID or human-friendly name (e.g. "ci-42").
    pub workflow_id: String,
    /// Filter to a single step name.
    #[arg(long)]
    pub step: Option<String>,
    /// Follow mode (`tail -f`).
    #[arg(short, long)]
    pub follow: bool,
}

pub async fn run(args: LogsArgs, mut client: AuthClient) -> Result<()> {
    let mut stream = client
        .stream_logs(StreamLogsRequest {
            workflow_id: args.workflow_id.clone(),
            step_name: args.step.unwrap_or_default(),
            follow: args.follow,
        })
        .await?
        .into_inner();

    while let Some(entry) = stream.next().await {
        let entry = entry?;
        let prefix = if entry.step_name.is_empty() {
            String::new()
        } else {
            format!("[{}] ", entry.step_name)
        };
        let line = String::from_utf8_lossy(&entry.data);
        let line = line.trim_end_matches('\n');
        let stream_kind = LogStream::try_from(entry.stream).unwrap_or(LogStream::Unspecified);
        if matches!(stream_kind, LogStream::Stderr) {
            eprintln!("{prefix}{line}");
        } else {
            println!("{prefix}{line}");
        }
    }
    Ok(())
}
