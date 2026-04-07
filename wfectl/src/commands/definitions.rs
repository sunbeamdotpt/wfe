//! `wfectl definitions list` -- list registered workflow definitions.

use anyhow::Result;
use clap::{Args, Subcommand};
use wfe_server_protos::wfe::v1::ListDefinitionsRequest;

use crate::client::AuthClient;
use crate::output::{OutputFormat, render_table};

#[derive(Debug, Args)]
pub struct DefinitionsArgs {
    #[command(subcommand)]
    pub cmd: DefinitionsCmd,
}

#[derive(Debug, Subcommand)]
pub enum DefinitionsCmd {
    /// List all registered workflow definitions.
    List,
}

pub async fn run(
    args: DefinitionsArgs,
    mut client: AuthClient,
    format: OutputFormat,
) -> Result<()> {
    match args.cmd {
        DefinitionsCmd::List => {
            let resp = client
                .list_definitions(ListDefinitionsRequest {})
                .await?
                .into_inner();

            if matches!(format, OutputFormat::Json) {
                let json = serde_json::json!({
                    "definitions": resp.definitions.iter().map(|d| serde_json::json!({
                        "id": d.id,
                        "name": d.name,
                        "version": d.version,
                        "description": d.description,
                        "step_count": d.step_count,
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&json)?);
            } else {
                let rows: Vec<Vec<String>> = resp
                    .definitions
                    .iter()
                    .map(|d| {
                        // Fall back to the slug id when no display name is set
                        // so the Name column is always populated.
                        let display = if d.name.is_empty() {
                            d.id.clone()
                        } else {
                            d.name.clone()
                        };
                        vec![
                            display,
                            d.id.clone(),
                            d.version.to_string(),
                            d.step_count.to_string(),
                            d.description.clone(),
                        ]
                    })
                    .collect();
                println!(
                    "{}",
                    render_table(
                        &["Name", "ID", "Version", "Steps", "Description"],
                        &rows
                    )
                );
                println!("{} definition(s)", resp.definitions.len());
            }
        }
    }
    Ok(())
}
