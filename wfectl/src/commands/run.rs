//! `wfectl run <target>` -- start a new workflow instance.
//!
//! If `target` is a path to a YAML file (or ends with `.yaml`/`.yml`), the
//! workflow is compiled and executed locally using an embedded WorkflowHost
//! with SQLite persistence. Otherwise `target` is treated as a remote workflow
//! definition ID and the command talks to `wfe-server` via gRPC.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use wfe::WorkflowHostBuilder;
use wfe_core::local_artifact_store::LocalArtifactStore;
use wfe_core::models::{PointerStatus, WorkflowStatus, artifact_ref_value};
use wfe_core::traits::ArtifactStore;
use wfe_core::test_support::{InMemoryLockProvider, InMemoryQueueProvider};
use wfe_server_protos::wfe::v1::StartWorkflowRequest;
use wfe_sqlite::SqlitePersistenceProvider;

use crate::client::AuthClient;
use crate::output::{OutputFormat, fmt_time, render_kv};
use crate::struct_util::json_object_to_struct;

#[derive(Debug, Args)]
/// Runargs.
pub struct RunArgs {
    /// Workflow definition ID or path to a workflow YAML file.
    pub target: String,
    /// Workflow version (default: 1).
    #[arg(long, default_value_t = 1)]
    pub version: u32,
    /// Human-friendly name for this instance. Must be unique across all
    /// workflow instances. Leave unset to let the server auto-assign
    /// `{definition_id}-{N}` using a per-definition monotonic counter.
    #[arg(long)]
    pub name: Option<String>,

    // --- shared flags ---
    /// Workflow input data as `key=value` pairs. Values are parsed as JSON
    /// where possible (numbers, booleans, null, objects, arrays); otherwise
    /// treated as strings. Can be specified multiple times.
    #[arg(short = 'i', long = "input", value_parser = parse_kv)]
    pub inputs: Vec<(String, String)>,
    /// Config interpolation values for YAML execution: `key=value`.
    /// Resolves `((var))` placeholders in the workflow definition.
    /// Can be specified multiple times.
    #[arg(long = "config", short = 'c', value_parser = parse_kv)]
    pub config: Vec<(String, String)>,

    // --- local-only flags ---
    /// Path to the SQLite database for local execution. Defaults to the
    /// platform data directory (e.g. ~/.local/share/wfectl/workflows.db on
    /// Linux, ~/Library/Application Support/wfectl/workflows.db on macOS).
    #[arg(long)]
    pub db: Option<PathBuf>,
    /// Timeout in seconds for local execution (default: 3600).
    #[arg(long, default_value_t = 3600)]
    pub timeout: u64,
}

fn parse_kv(raw: &str) -> Result<(String, String), String> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| format!("expected key=value, got: {raw}"))
}

/// Parse an input value: try JSON first, fall back to plain string.
fn parse_input_value(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

/// Build workflow data JSON object from `-i key=value` pairs.
/// File paths that exist on disk are tar-gzipped and stored as artifact
/// references instead of raw strings.
async fn build_data(
    inputs: &[(String, String)],
    artifact_store: Option<&LocalArtifactStore>,
) -> Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in inputs {
        let path = Path::new(v);
        if path.exists() {
            let store = artifact_store
                .context("artifact store required for file inputs")?;
            let digest = tar_and_store(path, store).await?;
            map.insert(k.clone(), artifact_ref_value(&digest));
        } else {
            map.insert(k.clone(), parse_input_value(v));
        }
    }
    Ok(serde_json::Value::Object(map))
}

/// Tar-gzip a file or directory and store it in the artifact store.
async fn tar_and_store(path: &Path, store: &LocalArtifactStore) -> Result<String> {
    let buf = tokio::task::spawn_blocking({
        let path = path.to_path_buf();
        move || {
            let mut buf = Vec::new();
            {
                let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
                let mut tar = tar::Builder::new(gz);
                if path.is_file() {
                    tar.append_path_with_name(&path, path.file_name().unwrap_or_default())?;
                } else {
                    tar.append_dir_all(".", &path)?;
                }
                tar.finish()?;
            }
            Ok::<Vec<u8>, std::io::Error>(buf)
        }
    })
    .await
    .context("tar task panicked")??;

    let aref = store
        .put(Box::pin(tokio::io::BufReader::new(std::io::Cursor::new(buf))))
        .await
        .context("failed to store artifact")?;
    Ok(aref.digest)
}

pub fn is_local_target(target: &str) -> bool {
    let path = Path::new(target);
    path.exists()
        || target.ends_with(".yaml")
        || target.ends_with(".yml")
}

fn default_db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("wfectl/workflows.db")
}

fn default_artifact_store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("wfectl/artifacts")
}

/// Run.
pub async fn run(args: RunArgs, client: Option<AuthClient>, format: OutputFormat) -> Result<()> {
    if is_local_target(&args.target) {
        run_local(args, format).await
    } else {
        let client = client.context("remote run requires a server connection")?;
        run_remote(args, client, format).await
    }
}

async fn run_remote(args: RunArgs, mut client: AuthClient, format: OutputFormat) -> Result<()> {
    // Remote file inputs are not yet supported.
    for (_, v) in &args.inputs {
        if Path::new(v).exists() {
            anyhow::bail!("file inputs are not yet supported for remote execution");
        }
    }
    let json_value = build_data(&args.inputs, None).await?;
    let data = json_object_to_struct(&json_value);

    let resp = client
        .start_workflow(StartWorkflowRequest {
            definition_id: args.target.clone(),
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
            "definition_id": args.target,
            "version": args.version,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!("Started workflow: {}", resp.name);
        println!("  ID:         {}", resp.workflow_id);
        println!("  Definition: {} v{}", args.target, args.version);
    }
    Ok(())
}

async fn run_local(args: RunArgs, format: OutputFormat) -> Result<()> {
    let yaml = std::fs::read_to_string(&args.target)
        .with_context(|| format!("failed to read {}", args.target))?;

    let config: HashMap<String, serde_json::Value> = args
        .config
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();

    let compiled = wfe_yaml::load_workflow_from_str(&yaml, &config)
        .with_context(|| format!("YAML compilation failed for {}", args.target))?;

    if compiled.is_empty() {
        anyhow::bail!("no workflows found in {}", args.target);
    }

    let target_id = match args.target.strip_suffix(".yaml").or_else(|| args.target.strip_suffix(".yml")) {
        // When target is a file, try to infer the definition id from the file name
        // if there's exactly one workflow in the file; otherwise require explicit
        // disambiguation.
        _ if compiled.len() == 1 => compiled[0].definition.id.clone(),
        _ => {
            let ids: Vec<String> = compiled.iter().map(|c| c.definition.id.clone()).collect();
            anyhow::bail!(
                "file contains multiple workflows; pass --definition-id with one of: {}",
                ids.join(", ")
            );
        }
    };

    let mut target_workflow = None;
    for cw in compiled {
        if cw.definition.id == target_id {
            target_workflow = Some(cw);
            break;
        }
    }
    let mut target_workflow = target_workflow
        .with_context(|| format!("definition '{target_id}' not found in {}", args.target))?;

    let db_path = args.db.unwrap_or_else(default_db_path);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create db dir {}", parent.display()))?;
    }

    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let persistence = SqlitePersistenceProvider::new(&db_url)
        .await
        .map_err(|e| anyhow::anyhow!("failed to open sqlite db at {}: {e}", db_path.display()))?;

    let artifact_store_path = default_artifact_store_path();
    std::fs::create_dir_all(&artifact_store_path)
        .with_context(|| format!("failed to create artifact store dir {}", artifact_store_path.display()))?;
    let artifact_store = LocalArtifactStore::open(&artifact_store_path)
        .await
        .context("failed to open artifact store")?;

    let host = WorkflowHostBuilder::new()
        .use_persistence(Arc::new(persistence))
        .use_lock_provider(Arc::new(InMemoryLockProvider::new()))
        .use_queue_provider(Arc::new(InMemoryQueueProvider::new()))
        .use_artifact_store(Arc::new(artifact_store.clone()))
        .build()
        .context("failed to build workflow host")?;

    host.start().await.context("failed to start workflow host")?;

    let factories = std::mem::take(&mut target_workflow.step_factories);
    for (key, factory) in factories {
        host.register_step_factory(&key, move || factory()).await;
    }
    host.register_workflow_definition(target_workflow.definition.clone())
        .await;

    let data = build_data(&args.inputs, Some(&artifact_store))
        .await
        .context("failed to process inputs")?;

    let workflow_id = match args.name {
        Some(name) => {
            host.start_workflow_with_name(&target_id, args.version, data, Some(name))
                .await
        }
        None => host.start_workflow(&target_id, args.version, data).await,
    }
    .context("failed to start workflow")?;

    if matches!(format, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "workflow_id": workflow_id,
                "definition_id": target_id,
                "version": args.version,
                "status": "started",
            }))?
        );
    } else {
        println!("Started local workflow: {workflow_id}");
        println!("  Definition: {target_id} v{}", args.version);
        println!("  Database:   {}", db_path.display());
    }

    let timeout = Duration::from_secs(args.timeout);
    let deadline = tokio::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(500);

    let final_instance = loop {
        let instance = host.get_workflow(&workflow_id).await?;
        match instance.status {
            WorkflowStatus::Complete | WorkflowStatus::Terminated => break instance,
            _ if tokio::time::Instant::now() > deadline => {
                anyhow::bail!("timeout: workflow did not complete within {timeout:?}");
            }
            _ => tokio::time::sleep(poll_interval).await,
        }
    };

    host.stop().await;

    let completed_pointers = final_instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == PointerStatus::Complete)
        .count();

    if matches!(format, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "workflow_id": final_instance.id,
                "name": final_instance.name,
                "definition_id": final_instance.workflow_definition_id,
                "version": final_instance.version,
                "status": final_instance.status,
                "create_time": fmt_time(&final_instance.create_time),
                "complete_time": final_instance.complete_time.as_ref().map(fmt_time),
                "pointers": {
                    "total": final_instance.execution_pointers.len(),
                    "completed": completed_pointers,
                },
                "data": final_instance.data,
            }))?
        );
    } else {
        let rows = vec![
            ("Workflow ID", final_instance.id.clone()),
            ("Name", final_instance.name.clone()),
            ("Definition", format!("{} v{}", final_instance.workflow_definition_id, final_instance.version)),
            ("Status", format!("{:?}", final_instance.status)),
            ("Created", fmt_time(&final_instance.create_time)),
            (
                "Completed",
                final_instance
                    .complete_time
                    .as_ref()
                    .map(fmt_time)
                    .unwrap_or_default(),
            ),
            (
                "Pointers",
                format!(
                    "{} total, {} complete",
                    final_instance.execution_pointers.len(),
                    completed_pointers
                ),
            ),
        ];
        println!("\n{}", render_kv(&rows));

        if let Some(obj) = final_instance.data.as_object() {
            if !obj.is_empty() {
                println!("\nWorkflow data:");
                for (k, v) in obj {
                    println!("  {k}: {v}");
                }
            }
        }
    }

    if final_instance.status == WorkflowStatus::Terminated {
        anyhow::bail!("workflow terminated");
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

    #[test]
    fn default_db_path_resolves() {
        let path = default_db_path();
        assert!(path.to_string_lossy().contains("wfectl/workflows.db"));
    }

    #[test]
    fn is_local_target_detects_yaml() {
        assert!(is_local_target("workflow.yaml"));
        assert!(is_local_target("workflow.yml"));
        assert!(!is_local_target("ci-pipeline"));
    }
}
