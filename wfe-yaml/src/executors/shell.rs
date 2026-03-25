use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};
use wfe_core::WfeError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    pub run: String,
    pub shell: String,
    pub env: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub timeout_ms: Option<u64>,
}

pub struct ShellStep {
    config: ShellConfig,
}

impl ShellStep {
    pub fn new(config: ShellConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl StepBody for ShellStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let mut cmd = tokio::process::Command::new(&self.config.shell);
        cmd.arg("-c").arg(&self.config.run);

        // Inject workflow data as UPPER_CASE env vars (top-level keys only).
        if let Some(data_obj) = context.workflow.data.as_object() {
            for (key, value) in data_obj {
                let env_key = key.to_uppercase();
                let env_val = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                cmd.env(&env_key, &env_val);
            }
        }

        // Add extra env from config.
        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        // Set working directory if specified.
        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Execute with optional timeout.
        let output = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, cmd.output()).await {
                Ok(result) => result.map_err(|e| WfeError::StepExecution(format!("Failed to spawn shell command: {e}")))?,
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "Shell command timed out after {}ms",
                        timeout_ms
                    )));
                }
            }
        } else {
            cmd.output()
                .await
                .map_err(|e| WfeError::StepExecution(format!("Failed to spawn shell command: {e}")))?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(WfeError::StepExecution(format!(
                "Shell command exited with code {code}\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        // Parse ##wfe[output name=value] lines from stdout.
        let mut outputs = serde_json::Map::new();
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("##wfe[output ")
                && let Some(rest) = rest.strip_suffix(']')
                && let Some(eq_pos) = rest.find('=')
            {
                let name = rest[..eq_pos].trim().to_string();
                let value = rest[eq_pos + 1..].to_string();
                outputs.insert(name, serde_json::Value::String(value));
            }
        }

        // Add raw stdout under the step name.
        let step_name = context
            .step
            .name
            .as_deref()
            .unwrap_or("unknown");
        outputs.insert(
            format!("{step_name}.stdout"),
            serde_json::Value::String(stdout.clone()),
        );
        outputs.insert(
            format!("{step_name}.stderr"),
            serde_json::Value::String(stderr),
        );

        Ok(ExecutionResult {
            proceed: true,
            output_data: Some(serde_json::Value::Object(outputs)),
            ..Default::default()
        })
    }
}
