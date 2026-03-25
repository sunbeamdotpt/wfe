use async_trait::async_trait;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};
use wfe_core::WfeError;

use super::config::DenoConfig;
use super::ops::workflow::StepOutputs;
use super::runtime::create_runtime;

/// A workflow step that executes JavaScript inside a Deno runtime.
pub struct DenoStep {
    config: DenoConfig,
}

impl DenoStep {
    pub fn new(config: DenoConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl StepBody for DenoStep {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let step_name = context
            .step
            .name
            .as_deref()
            .unwrap_or("unknown")
            .to_string();

        let workflow_data = context.workflow.data.clone();

        // Resolve script source.
        let source = if let Some(ref script) = self.config.script {
            script.clone()
        } else if let Some(ref file_path) = self.config.file {
            std::fs::read_to_string(file_path).map_err(|e| {
                WfeError::StepExecution(format!(
                    "Failed to read deno script file '{}': {}",
                    file_path, e
                ))
            })?
        } else {
            return Err(WfeError::StepExecution(
                "Deno step must have either 'script' or 'file' configured".to_string(),
            ));
        };

        let config = self.config.clone();
        let timeout_ms = self.config.timeout_ms;

        // JsRuntime is !Send, so we run it on a dedicated thread with its own
        // single-threaded tokio runtime.
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| {
                    WfeError::StepExecution(format!("Failed to build tokio runtime: {e}"))
                })?;

            rt.block_on(async move {
                run_script_inner(&config, workflow_data, &step_name, &source, timeout_ms)
                    .await
            })
        });

        // Wait for the thread.
        tokio::task::spawn_blocking(move || {
            handle
                .join()
                .map_err(|_| WfeError::StepExecution("Deno thread panicked".to_string()))?
        })
        .await
        .map_err(|e| WfeError::StepExecution(format!("Join error: {e}")))?
    }
}

async fn run_script_inner(
    config: &DenoConfig,
    workflow_data: serde_json::Value,
    step_name: &str,
    source: &str,
    timeout_ms: Option<u64>,
) -> wfe_core::Result<ExecutionResult> {
    let mut runtime = create_runtime(config, workflow_data, step_name)?;

    // If a timeout is configured, set up a V8 termination timer.
    // This handles synchronous infinite loops that never yield to the event loop.
    let _timeout_guard = timeout_ms.map(|ms| {
        let isolate_handle = runtime.v8_isolate().thread_safe_handle();
        let duration = std::time::Duration::from_millis(ms);
        std::thread::spawn(move || {
            std::thread::sleep(duration);
            isolate_handle.terminate_execution();
        })
    });

    // Execute the script.
    runtime
        .execute_script("<wfe>", source.to_string())
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("terminated") {
                WfeError::StepExecution(format!(
                    "Deno script timed out after {}ms",
                    timeout_ms.unwrap_or(0)
                ))
            } else {
                WfeError::StepExecution(format!("Deno script error: {e}"))
            }
        })?;

    // Run the event loop to completion.
    runtime
        .run_event_loop(Default::default())
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("terminated") {
                WfeError::StepExecution(format!(
                    "Deno script timed out after {}ms",
                    timeout_ms.unwrap_or(0)
                ))
            } else {
                WfeError::StepExecution(format!("Deno event loop error: {e}"))
            }
        })?;

    // Extract outputs from OpState.
    let outputs = {
        let state = runtime.op_state();
        let mut state = state.borrow_mut();
        let step_outputs = state.borrow_mut::<StepOutputs>();
        std::mem::take(&mut step_outputs.map)
    };

    let output_data = if outputs.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(
            outputs
                .into_iter()
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        ))
    };

    Ok(ExecutionResult {
        proceed: true,
        output_data,
        ..Default::default()
    })
}
