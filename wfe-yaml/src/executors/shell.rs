use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use wfe_core::WfeError;
use wfe_core::local_artifact_store::extract_artifact_to_dir;
use wfe_core::models::{ExecutionResult, parse_artifact_ref};
use wfe_core::traits::step::{StepBody, StepExecutionContext};


#[derive(Debug, Clone, Serialize, Deserialize)]
/// Shellconfig.
pub struct ShellConfig {
    /// Run.
    pub run: String,
    /// Shell.
    pub shell: String,
    /// Env.
    pub env: HashMap<String, String>,
    /// Working dir.
    pub working_dir: Option<String>,
    /// Timeout ms.
    pub timeout_ms: Option<u64>,
    /// Artifact inputs to mount before running.
    /// Map of artifact name → mount point path.
    pub inputs: Option<HashMap<String, String>>,
}

/// Shellstep.
pub struct ShellStep {
    config: ShellConfig,
    /// Tracks artifact mount points created during mount_artifacts for cleanup.
    /// Map of input name → mount path.
    mount_points: HashMap<String, PathBuf>,
}

impl ShellStep {
    pub fn new(config: ShellConfig) -> Self {
        Self {
            config,
            mount_points: HashMap::new(),
        }
    }

    fn build_command(&self, context: &StepExecutionContext<'_>) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.config.shell);
        cmd.arg("-c").arg(&self.config.run);

        // Inject workflow data as UPPER_CASE env vars (top-level keys only).
        // Skip keys that would override security-sensitive environment variables.
        const BLOCKED_KEYS: &[&str] = &[
            "PATH",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "HOME",
            "SHELL",
            "USER",
            "LOGNAME",
            "TERM",
        ];
        if let Some(data_obj) = context.workflow.data.as_object() {
            for (key, value) in data_obj {
                let env_key = key.to_uppercase();
                if BLOCKED_KEYS.contains(&env_key.as_str()) {
                    continue;
                }
                // Skip artifact references — they are resolved to INPUT_* paths.
                if parse_artifact_ref(value).is_some() {
                    continue;
                }
                let env_val = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                cmd.env(&env_key, &env_val);
            }
        }

        // Inject INPUT_<NAME> env vars for mounted artifacts.
        for (name, mount_point) in &self.mount_points {
            cmd.env(format!("INPUT_{}", name.to_uppercase()), mount_point.as_os_str());
        }

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        cmd
    }

    /// Run with streaming output via LogSink.
    ///
    /// Reads stdout and stderr line-by-line, streaming each line to the
    /// LogSink as it's produced. Uses `tokio::select!` to interleave both
    /// streams without spawning tasks (avoids lifetime issues with &dyn LogSink).
    async fn run_streaming(
        &self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<(String, String, i32)> {
        use tokio::io::{AsyncBufReadExt, BufReader};
        use wfe_core::traits::{LogChunk, LogStreamType};

        let log_sink = context.log_sink.unwrap();
        let workflow_id = context.workflow.id.clone();
        let definition_id = context.workflow.workflow_definition_id.clone();
        let step_id = context.step.id;
        let step_name = context
            .step
            .name
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let mut cmd = self.build_command(context);
        let mut child = cmd
            .spawn()
            .map_err(|e| WfeError::StepExecution(format!("Failed to spawn shell command: {e}")))?;

        let stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| WfeError::StepExecution("failed to capture stdout pipe".to_string()))?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| WfeError::StepExecution("failed to capture stderr pipe".to_string()))?;
        let mut stdout_lines = BufReader::new(stdout_pipe).lines();
        let mut stderr_lines = BufReader::new(stderr_pipe).lines();

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut stdout_done = false;
        let mut stderr_done = false;

        // Interleave stdout/stderr reads with optional timeout.
        let read_future = async {
            while !stdout_done || !stderr_done {
                tokio::select! {
                    line = stdout_lines.next_line(), if !stdout_done => {
                        match line {
                            Ok(Some(line)) => {
                                log_sink.write_chunk(LogChunk {
                                    workflow_id: workflow_id.clone(),
                                    definition_id: definition_id.clone(),
                                    step_id,
                                    step_name: step_name.clone(),
                                    stream: LogStreamType::Stdout,
                                    data: format!("{line}\n").into_bytes(),
                                    timestamp: chrono::Utc::now(),
                                }).await;
                                stdout_buf.push(line);
                            }
                            _ => stdout_done = true,
                        }
                    }
                    line = stderr_lines.next_line(), if !stderr_done => {
                        match line {
                            Ok(Some(line)) => {
                                log_sink.write_chunk(LogChunk {
                                    workflow_id: workflow_id.clone(),
                                    definition_id: definition_id.clone(),
                                    step_id,
                                    step_name: step_name.clone(),
                                    stream: LogStreamType::Stderr,
                                    data: format!("{line}\n").into_bytes(),
                                    timestamp: chrono::Utc::now(),
                                }).await;
                                stderr_buf.push(line);
                            }
                            _ => stderr_done = true,
                        }
                    }
                }
            }
            child.wait().await
        };

        let status = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, read_future).await {
                Ok(result) => result.map_err(|e| {
                    WfeError::StepExecution(format!("Failed to wait for shell command: {e}"))
                })?,
                Err(_) => {
                    // Kill the child on timeout.
                    let _ = child.kill().await;
                    return Err(WfeError::StepExecution(format!(
                        "Shell command timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            read_future.await.map_err(|e| {
                WfeError::StepExecution(format!("Failed to wait for shell command: {e}"))
            })?
        };

        let mut stdout = stdout_buf.join("\n");
        let mut stderr = stderr_buf.join("\n");
        if !stdout.is_empty() {
            stdout.push('\n');
        }
        if !stderr.is_empty() {
            stderr.push('\n');
        }

        Ok((stdout, stderr, status.code().unwrap_or(-1)))
    }

    /// Run with buffered output (original path, no LogSink).
    async fn run_buffered(
        &self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<(String, String, i32)> {
        let mut cmd = self.build_command(context);

        let output = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, cmd.output()).await {
                Ok(result) => result.map_err(|e| {
                    WfeError::StepExecution(format!("Failed to spawn shell command: {e}"))
                })?,
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "Shell command timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            cmd.output().await.map_err(|e| {
                WfeError::StepExecution(format!("Failed to spawn shell command: {e}"))
            })?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let code = output.status.code().unwrap_or(-1);

        Ok((stdout, stderr, code))
    }
}

#[async_trait]
impl StepBody for ShellStep {
    async fn mount_artifacts(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<()> {
        let Some(ref inputs) = self.config.inputs else {
            return Ok(());
        };

        if inputs.is_empty() {
            return Ok(());
        }

        let Some(volume) = context.artifact_volume else {
            return Err(WfeError::StepExecution(
                "artifact volume required but not provided".to_string(),
            ));
        };

        for (name, mount_point_str) in inputs {
            let mount_point = PathBuf::from(mount_point_str);
            // If relative, resolve under a temp dir.
            let mount_point = if mount_point.is_absolute() {
                mount_point
            } else {
                let tmp = std::env::temp_dir().join(format!("wfe-shell-{}", uuid::Uuid::new_v4()));
                tmp.join(&mount_point)
            };

            let bytes = volume
                .get(name)
                .cloned()
                .ok_or_else(|| WfeError::StepExecution(format!("artifact '{name}' not in volume")))?;

            tokio::task::spawn_blocking({
                let mount_point = mount_point.clone();
                move || extract_artifact_to_dir(std::io::Cursor::new(bytes), &mount_point)
            })
            .await
            .map_err(|e| WfeError::StepExecution(format!("extract task panicked: {e}")))??;

            self.mount_points.insert(name.clone(), mount_point);
        }

        Ok(())
    }

    async fn unmount_artifacts(
        &mut self,
        _context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<()> {
        for (_, mount_point) in self.mount_points.drain() {
            let _ = tokio::fs::remove_dir_all(&mount_point).await;
        }
        Ok(())
    }

    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let (stdout, stderr, exit_code) = if context.log_sink.is_some() {
            self.run_streaming(context).await?
        } else {
            self.run_buffered(context).await?
        };

        if exit_code != 0 {
            return Err(WfeError::StepExecution(format!(
                "Shell command exited with code {exit_code}\nstdout: {stdout}\nstderr: {stderr}"
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
                let raw_value = rest[eq_pos + 1..].to_string();
                let value = match raw_value.as_str() {
                    "true" => serde_json::Value::Bool(true),
                    "false" => serde_json::Value::Bool(false),
                    "null" => serde_json::Value::Null,
                    s if s.parse::<i64>().is_ok() => {
                        serde_json::Value::Number(s.parse::<i64>().unwrap().into())
                    }
                    s if s.parse::<f64>().is_ok() => {
                        serde_json::json!(s.parse::<f64>().unwrap())
                    }
                    _ => serde_json::Value::String(raw_value),
                };
                outputs.insert(name, value);
            }
        }

        let step_name = context.step.name.as_deref().unwrap_or("unknown");
        outputs.insert(
            format!("{step_name}.stdout"),
            serde_json::Value::String(stdout),
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
