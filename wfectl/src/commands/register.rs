//! `wfectl register <yaml-file>` -- register one or more workflow definitions.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use wfe_server_protos::wfe::v1::RegisterWorkflowRequest;

use crate::client::AuthClient;
use crate::output::{OutputFormat, render_table};

#[derive(Debug, Args)]
/// Registerargs.
pub struct RegisterArgs {
    /// Path to a workflow YAML file.
    pub file: PathBuf,
    /// Config interpolation values: `key=value`. Repeatable.
    #[arg(long = "config", short = 'c', value_parser = parse_kv)]
    pub config: Vec<(String, String)>,
}

fn parse_kv(raw: &str) -> Result<(String, String), String> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got: {raw}"))
}

/// Run.
pub async fn run(args: RegisterArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
    let yaml = std::fs::read_to_string(&args.file)
        .with_context(|| format!("failed to read {}", args.file.display()))?;

    let config: HashMap<String, String> = args.config.into_iter().collect();

    let resp = client
        .register_workflow(RegisterWorkflowRequest { yaml, config })
        .await?
        .into_inner();

    if matches!(format, OutputFormat::Json) {
        let json = serde_json::json!({
            "definitions": resp.definitions.iter().map(|d| serde_json::json!({
                "id": d.definition_id,
                "name": d.name,
                "version": d.version,
                "step_count": d.step_count,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        let rows: Vec<Vec<String>> = resp
            .definitions
            .iter()
            .map(|d| {
                let display = if d.name.is_empty() {
                    d.definition_id.clone()
                } else {
                    d.name.clone()
                };
                vec![
                    display,
                    d.definition_id.clone(),
                    d.version.to_string(),
                    d.step_count.to_string(),
                ]
            })
            .collect();
        println!(
            "{}",
            render_table(&["Name", "ID", "Version", "Steps"], &rows)
        );
        println!("Registered {} workflow(s)", resp.definitions.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kv_valid() {
        let (k, v) = parse_kv("foo=bar").unwrap();
        assert_eq!(k, "foo");
        assert_eq!(v, "bar");
    }

    #[test]
    fn parse_kv_with_equals_in_value() {
        let (k, v) = parse_kv("k=a=b=c").unwrap();
        assert_eq!(k, "k");
        assert_eq!(v, "a=b=c");
    }

    #[test]
    fn parse_kv_missing_equals() {
        assert!(parse_kv("invalid").is_err());
    }
}
