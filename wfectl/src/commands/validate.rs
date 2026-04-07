//! `wfectl validate <FILE>` -- locally compile a workflow YAML file.
//!
//! Validation runs in-process via `wfe_yaml::load_workflow_from_str`, which
//! is the exact same compile path the server uses at registration time. The
//! wfectl crate enables the full executor feature set (kubernetes, deno,
//! buildkit, containerd, rustlang) so every step type is recognized. No
//! server round-trip, no auth required — giving users instant feedback
//! before they push to a shared host.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use crate::output::{OutputFormat, render_table};

#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Path to a workflow YAML file.
    pub file: PathBuf,
    /// Config interpolation values: `key=value`. Repeatable. Mirrors
    /// `wfectl register` so validation sees the same interpolated text.
    #[arg(long = "config", short = 'c', value_parser = parse_kv)]
    pub config: Vec<(String, String)>,
}

fn parse_kv(raw: &str) -> Result<(String, String), String> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got: {raw}"))
}

pub async fn run(args: ValidateArgs, format: OutputFormat) -> Result<()> {
    let yaml = std::fs::read_to_string(&args.file)
        .with_context(|| format!("failed to read {}", args.file.display()))?;

    // wfe_yaml takes a JSON-valued config map because YAML interpolation
    // supports non-string types; wrap every CLI-supplied value as a string.
    let config: HashMap<String, serde_json::Value> = args
        .config
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();

    let compiled = wfe_yaml::load_workflow_from_str(&yaml, &config)
        .with_context(|| format!("YAML compilation failed for {}", args.file.display()))?;

    if matches!(format, OutputFormat::Json) {
        let json = serde_json::json!({
            "file": args.file.display().to_string(),
            "definitions": compiled.iter().map(|c| serde_json::json!({
                "id": c.definition.id,
                "name": c.definition.name,
                "version": c.definition.version,
                "description": c.definition.description,
                "step_count": c.definition.steps.len(),
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        let rows: Vec<Vec<String>> = compiled
            .iter()
            .map(|c| {
                let display = c
                    .definition
                    .name
                    .clone()
                    .unwrap_or_else(|| c.definition.id.clone());
                vec![
                    display,
                    c.definition.id.clone(),
                    c.definition.version.to_string(),
                    c.definition.steps.len().to_string(),
                ]
            })
            .collect();
        println!(
            "{}",
            render_table(&["Name", "ID", "Version", "Steps"], &rows)
        );
        println!("✓ {} valid workflow(s)", compiled.len());
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
    fn parse_kv_missing_equals() {
        assert!(parse_kv("invalid").is_err());
    }

    #[tokio::test]
    async fn validate_accepts_simple_workflow() {
        // Use a `kubernetes` step because the validator enables the full
        // executor feature set. We don't actually run the step, only
        // compile the definition.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"
workflow:
  id: simple
  name: "Simple Test Workflow"
  version: 1
  steps:
    - name: hello
      type: kubernetes
      config:
        image: alpine:3.19
        run: echo hello
"#,
        )
        .unwrap();

        let args = ValidateArgs {
            file: tmp.path().to_path_buf(),
            config: vec![],
        };
        run(args, OutputFormat::Json).await.unwrap();
    }

    #[tokio::test]
    async fn validate_rejects_unknown_step_type() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            tmp.path(),
            r#"
workflow:
  id: broken
  version: 1
  steps:
    - name: nope
      type: not-a-real-step-type
      config: {}
"#,
        )
        .unwrap();

        let args = ValidateArgs {
            file: tmp.path().to_path_buf(),
            config: vec![],
        };
        let err = run(args, OutputFormat::Table).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Unknown step type") || msg.contains("compilation failed"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_rejects_missing_file() {
        let args = ValidateArgs {
            file: PathBuf::from("/definitely/does/not/exist.yaml"),
            config: vec![],
        };
        let err = run(args, OutputFormat::Table).await.unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("failed to read"), "unexpected error: {msg}");
    }
}
