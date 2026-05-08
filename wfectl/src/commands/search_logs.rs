//! `wfectl search-logs <query>` -- full-text search log lines.

use anyhow::Result;
use clap::{Args, ValueEnum};
use wfe_server_protos::wfe::v1::{LogStream, SearchLogsRequest};

use crate::client::AuthClient;
use crate::output::{OutputFormat, fmt_proto_time, render_table};

#[derive(Debug, Args)]
/// Searchlogsargs.
pub struct SearchLogsArgs {
    /// Full-text search query.
    pub query: String,
    /// Filter to a specific workflow.
    #[arg(long)]
    pub workflow: Option<String>,
    /// Filter to a specific step.
    #[arg(long)]
    pub step: Option<String>,
    /// Filter to stdout or stderr.
    #[arg(long)]
    pub stream: Option<StreamFilter>,
    /// Maximum number of results.
    #[arg(long, default_value_t = 50)]
    pub limit: u64,
    /// Skip the first N results.
    #[arg(long, default_value_t = 0)]
    pub skip: u64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
/// Streamfilter.
pub enum StreamFilter {
    /// Stdout.
    Stdout,
    /// Stderr.
    Stderr,
}

impl From<StreamFilter> for LogStream {
    fn from(s: StreamFilter) -> Self {
        match s {
            StreamFilter::Stdout => LogStream::Stdout,
            StreamFilter::Stderr => LogStream::Stderr,
        }
    }
}

/// Run.
pub async fn run(args: SearchLogsArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
    let stream_filter: LogStream = args
        .stream
        .map(Into::into)
        .unwrap_or(LogStream::Unspecified);
    let resp = client
        .search_logs(SearchLogsRequest {
            query: args.query,
            workflow_id: args.workflow.unwrap_or_default(),
            step_name: args.step.unwrap_or_default(),
            stream_filter: stream_filter as i32,
            skip: args.skip,
            take: args.limit,
        })
        .await?
        .into_inner();

    if matches!(format, OutputFormat::Json) {
        let json = serde_json::json!({
            "total": resp.total,
            "results": resp.results.iter().map(|r| serde_json::json!({
                "workflow_id": r.workflow_id,
                "definition_id": r.definition_id,
                "step_name": r.step_name,
                "stream": r.stream,
                "line": r.line,
                "timestamp": r.timestamp.as_ref().map(fmt_proto_time),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        let rows: Vec<Vec<String>> = resp
            .results
            .iter()
            .map(|r| {
                vec![
                    r.timestamp.as_ref().map(fmt_proto_time).unwrap_or_default(),
                    r.workflow_id.clone(),
                    r.step_name.clone(),
                    r.line.clone(),
                ]
            })
            .collect();
        println!(
            "{}",
            render_table(&["Time", "Workflow", "Step", "Line"], &rows)
        );
        println!("{} of {} result(s)", resp.results.len(), resp.total);
    }
    Ok(())
}
