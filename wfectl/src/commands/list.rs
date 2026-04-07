//! `wfectl list` -- search workflow instances.

use anyhow::Result;
use clap::{Args, ValueEnum};
use wfe_server_protos::wfe::v1::{SearchWorkflowsRequest, WorkflowStatus};

use crate::client::AuthClient;
use crate::output::{OutputFormat, fmt_proto_time, render_table};

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Free-text query.
    #[arg(long)]
    pub query: Option<String>,
    /// Filter by status.
    #[arg(long)]
    pub status: Option<StatusFilter>,
    /// Maximum number of results.
    #[arg(long, default_value_t = 50)]
    pub limit: u64,
    /// Skip the first N results.
    #[arg(long, default_value_t = 0)]
    pub skip: u64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum StatusFilter {
    Runnable,
    Suspended,
    Complete,
    Terminated,
}

impl From<StatusFilter> for WorkflowStatus {
    fn from(s: StatusFilter) -> Self {
        match s {
            StatusFilter::Runnable => WorkflowStatus::Runnable,
            StatusFilter::Suspended => WorkflowStatus::Suspended,
            StatusFilter::Complete => WorkflowStatus::Complete,
            StatusFilter::Terminated => WorkflowStatus::Terminated,
        }
    }
}

pub async fn run(args: ListArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
    let status: WorkflowStatus = args
        .status
        .map(Into::into)
        .unwrap_or(WorkflowStatus::Unspecified);
    let resp = client
        .search_workflows(SearchWorkflowsRequest {
            query: args.query.unwrap_or_default(),
            status_filter: status as i32,
            skip: args.skip,
            take: args.limit,
        })
        .await?
        .into_inner();

    if matches!(format, OutputFormat::Json) {
        let json = serde_json::json!({
            "total": resp.total,
            "results": resp.results.iter().map(|r| serde_json::json!({
                "id": r.id,
                "name": r.name,
                "definition_id": r.definition_id,
                "version": r.version,
                "status": r.status,
                "reference": r.reference,
                "description": r.description,
                "create_time": r.create_time.as_ref().map(fmt_proto_time),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        let rows: Vec<Vec<String>> = resp
            .results
            .iter()
            .map(|r| {
                vec![
                    r.name.clone(),
                    r.id.clone(),
                    format!("{} v{}", r.definition_id, r.version),
                    format!("{:?}", r.status),
                    r.create_time
                        .as_ref()
                        .map(fmt_proto_time)
                        .unwrap_or_default(),
                ]
            })
            .collect();
        println!(
            "{}",
            render_table(&["Name", "ID", "Definition", "Status", "Created"], &rows)
        );
        println!("{} of {} result(s)", resp.results.len(), resp.total);
    }
    Ok(())
}
