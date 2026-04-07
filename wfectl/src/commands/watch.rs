//! `wfectl watch [<workflow-id>]` -- stream lifecycle events.

use anyhow::Result;
use clap::Args;
use futures::StreamExt;
use wfe_server_protos::wfe::v1::{LifecycleEventType, WatchLifecycleRequest};

use crate::client::AuthClient;
use crate::output::fmt_proto_time;

#[derive(Debug, Args)]
pub struct WatchArgs {
    /// Optional workflow ID to filter to. Empty = all workflows.
    pub workflow_id: Option<String>,
}

pub async fn run(args: WatchArgs, mut client: AuthClient) -> Result<()> {
    let mut stream = client
        .watch_lifecycle(WatchLifecycleRequest {
            workflow_id: args.workflow_id.unwrap_or_default(),
        })
        .await?
        .into_inner();

    while let Some(event) = stream.next().await {
        let event = event?;
        let ts = event
            .event_time
            .as_ref()
            .map(fmt_proto_time)
            .unwrap_or_default();
        let event_type = format_event_type(event.event_type);
        let mut line = format!(
            "[{ts}] [{event_type}] workflow={} def={} v={}",
            event.workflow_id, event.definition_id, event.version
        );
        if !event.step_name.is_empty() {
            line.push_str(&format!(" step={}", event.step_name));
        }
        if !event.error_message.is_empty() {
            line.push_str(&format!(" error={}", event.error_message));
        }
        println!("{line}");
    }
    Ok(())
}

fn format_event_type(t: i32) -> &'static str {
    match LifecycleEventType::try_from(t).unwrap_or(LifecycleEventType::Unspecified) {
        LifecycleEventType::Started => "STARTED",
        LifecycleEventType::Completed => "COMPLETED",
        LifecycleEventType::Terminated => "TERMINATED",
        LifecycleEventType::Suspended => "SUSPENDED",
        LifecycleEventType::Resumed => "RESUMED",
        LifecycleEventType::Error => "ERROR",
        LifecycleEventType::StepStarted => "STEP_STARTED",
        LifecycleEventType::StepCompleted => "STEP_COMPLETED",
        LifecycleEventType::Unspecified => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_all_event_types() {
        assert_eq!(
            format_event_type(LifecycleEventType::Started as i32),
            "STARTED"
        );
        assert_eq!(
            format_event_type(LifecycleEventType::Completed as i32),
            "COMPLETED"
        );
        assert_eq!(
            format_event_type(LifecycleEventType::StepCompleted as i32),
            "STEP_COMPLETED"
        );
        assert_eq!(format_event_type(999), "UNKNOWN");
    }
}
