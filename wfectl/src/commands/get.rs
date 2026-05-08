//! `wfectl get <workflow-id>` -- fetch a workflow instance.

use anyhow::Result;
use clap::Args;
use wfe_server_protos::wfe::v1::GetWorkflowRequest;

use crate::client::AuthClient;
use crate::output::{OutputFormat, fmt_proto_time, render_kv, render_table};
use crate::struct_util::prost_struct_to_json;

#[derive(Debug, Args)]
/// Getargs.
pub struct GetArgs {
    /// Workflow instance identifier — either the UUID (`id`) or the
    /// human-friendly name (e.g. "ci-42"). The server resolves either form.
    pub workflow_id: String,
}

/// Run.
pub async fn run(args: GetArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
    let resp = client
        .get_workflow(GetWorkflowRequest {
            workflow_id: args.workflow_id.clone(),
        })
        .await?
        .into_inner();

    let instance = resp
        .instance
        .ok_or_else(|| anyhow::anyhow!("server returned empty instance"))?;

    if matches!(format, OutputFormat::Json) {
        let data = instance
            .data
            .as_ref()
            .map(prost_struct_to_json)
            .unwrap_or(serde_json::Value::Null);
        let json = serde_json::json!({
            "id": instance.id,
            "name": instance.name,
            "definition_id": instance.definition_id,
            "version": instance.version,
            "status": instance.status,
            "description": instance.description,
            "reference": instance.reference,
            "data": data,
            "create_time": instance.create_time.as_ref().map(fmt_proto_time),
            "complete_time": instance.complete_time.as_ref().map(fmt_proto_time),
            "execution_pointers": instance.execution_pointers.iter().map(|p| serde_json::json!({
                "id": p.id,
                "step_id": p.step_id,
                "step_name": p.step_name,
                "status": p.status,
                "active": p.active,
                "retry_count": p.retry_count,
                "start_time": p.start_time.as_ref().map(fmt_proto_time),
                "end_time": p.end_time.as_ref().map(fmt_proto_time),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    let mut fields = vec![
        ("Name", instance.name.clone()),
        ("ID", instance.id.clone()),
        (
            "Definition",
            format!("{} v{}", instance.definition_id, instance.version),
        ),
        ("Status", format!("{:?}", instance.status)),
    ];
    if !instance.description.is_empty() {
        fields.push(("Description", instance.description.clone()));
    }
    if !instance.reference.is_empty() {
        fields.push(("Reference", instance.reference.clone()));
    }
    if let Some(ts) = &instance.create_time {
        fields.push(("Created", fmt_proto_time(ts)));
    }
    if let Some(ts) = &instance.complete_time {
        fields.push(("Completed", fmt_proto_time(ts)));
    }
    let display: Vec<(&str, String)> = fields.iter().map(|(k, v)| (*k, v.clone())).collect();
    println!("{}", render_kv(&display));

    if !instance.execution_pointers.is_empty() {
        println!("\nExecution pointers:");
        let rows: Vec<Vec<String>> = instance
            .execution_pointers
            .iter()
            .map(|p| {
                vec![
                    p.step_name.clone(),
                    p.step_id.to_string(),
                    format!("{:?}", p.status),
                    if p.active { "yes".into() } else { "no".into() },
                    p.retry_count.to_string(),
                ]
            })
            .collect();
        println!(
            "{}",
            render_table(&["Step", "ID", "Status", "Active", "Retries"], &rows)
        );
    }
    Ok(())
}
