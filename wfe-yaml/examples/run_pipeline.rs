// =============================================================================
// WFE Self-Hosting CI Pipeline Runner
// =============================================================================
//
// Loads the multi-workflow CI pipeline from a YAML file and runs it to
// completion using the WFE engine with in-memory providers.
//
// Usage:
//   cargo run --example run_pipeline -p wfe-yaml -- workflows.yaml
//
// With config:
//   WFE_CONFIG='{"workspace_dir":"/path/to/wfe","registry":"sunbeam"}' \
//     cargo run --example run_pipeline -p wfe-yaml -- workflows.yaml

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use wfe::WorkflowHostBuilder;
use wfe::models::WorkflowStatus;
use wfe::test_support::{InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up tracing.
    tracing_subscriber::fmt()
        .with_target(false)
        .with_timer(tracing_subscriber::fmt::time::uptime())
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "wfe_core=info,wfe=info,run_pipeline=info".into()),
        )
        .init();

    // Read YAML path from args.
    let yaml_path = std::env::args()
        .nth(1)
        .expect("usage: run_pipeline <workflows.yaml>");

    // Read config from WFE_CONFIG env var (JSON map), merged over sensible defaults.
    let cwd = std::env::current_dir()?.to_string_lossy().to_string();

    // Defaults for every ((var)) referenced in the YAML.
    let mut config: HashMap<String, serde_json::Value> = HashMap::from([
        ("workspace_dir".into(), json!(cwd)),
        ("coverage_threshold".into(), json!(85)),
        ("registry".into(), json!("sunbeam")),
        ("git_remote".into(), json!("origin")),
        ("version".into(), json!("0.0.0")),
    ]);

    // Overlay user-provided config (WFE_CONFIG env var, JSON object).
    if let Ok(user_json) = std::env::var("WFE_CONFIG") {
        let user: HashMap<String, serde_json::Value> = serde_json::from_str(&user_json)?;
        config.extend(user);
    }

    let config_json = serde_json::to_string(&config)?;

    println!("Loading workflows from: {yaml_path}");
    println!("Config: {config_json}");

    // Load and compile all workflow definitions from the YAML file.
    let yaml_content = std::fs::read_to_string(&yaml_path)?;
    let workflows = wfe_yaml::load_workflow_from_str(&yaml_content, &config)?;

    println!("Compiled {} workflow(s):", workflows.len());
    for compiled in &workflows {
        println!(
            "  - {} v{}  ({} step factories)",
            compiled.definition.id,
            compiled.definition.version,
            compiled.step_factories.len(),
        );
    }

    // Build the host with in-memory providers.
    let persistence = Arc::new(InMemoryPersistenceProvider::default());
    let lock = Arc::new(InMemoryLockProvider::default());
    let queue = Arc::new(InMemoryQueueProvider::default());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence)
        .use_lock_provider(lock)
        .use_queue_provider(queue)
        .build()?;

    // Register all compiled workflows and their step factories.
    // We must move the factories out of the compiled workflows since
    // register_step_factory requires 'static closures.
    for mut compiled in workflows {
        let factories = std::mem::take(&mut compiled.step_factories);
        for (key, factory) in factories {
            host.register_step_factory(&key, move || factory()).await;
        }
        host.register_workflow_definition(compiled.definition).await;
    }

    // Start the engine.
    host.start().await?;
    println!("\nEngine started. Launching 'ci' workflow...\n");

    // Determine workspace_dir for initial data (use config value or cwd).
    let workspace_dir = config
        .get("workspace_dir")
        .and_then(|v| v.as_str())
        .unwrap_or(&cwd)
        .to_string();

    let data = json!({
        "workspace_dir": workspace_dir,
    });

    let workflow_id = host.start_workflow("ci", 1, data).await?;
    println!("Workflow instance: {workflow_id}");

    // Poll for completion with a 1-hour timeout.
    let timeout = Duration::from_secs(3600);
    let deadline = tokio::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(500);

    let final_instance = loop {
        let instance = host.get_workflow(&workflow_id).await?;
        match instance.status {
            WorkflowStatus::Complete | WorkflowStatus::Terminated => break instance,
            _ if tokio::time::Instant::now() > deadline => {
                eprintln!("Timeout: workflow did not complete within {timeout:?}");
                break instance;
            }
            _ => tokio::time::sleep(poll_interval).await,
        }
    };

    // Print final status.
    println!("\n========================================");
    println!("Pipeline status: {:?}", final_instance.status);
    println!(
        "Execution pointers: {} total, {} complete",
        final_instance.execution_pointers.len(),
        final_instance
            .execution_pointers
            .iter()
            .filter(|p| p.status == wfe::models::PointerStatus::Complete)
            .count()
    );

    // Print workflow data (contains outputs from all steps).
    if let Some(obj) = final_instance.data.as_object() {
        println!("\nKey outputs:");
        for key in [
            "version",
            "all_tests_passed",
            "coverage",
            "published",
            "released",
        ] {
            if let Some(val) = obj.get(key) {
                println!("  {key}: {val}");
            }
        }
    }
    println!("========================================");

    host.stop().await;
    println!("\nEngine stopped.");
    Ok(())
}
