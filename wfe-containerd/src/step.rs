use std::collections::HashMap;

use async_trait::async_trait;
use wfe_core::WfeError;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

use crate::config::ContainerdConfig;

pub struct ContainerdStep {
    config: ContainerdConfig,
}

impl ContainerdStep {
    pub fn new(config: ContainerdConfig) -> Self {
        Self { config }
    }

    /// Build the pull command arguments using the configured CLI binary.
    pub fn build_pull_command(&self) -> Vec<String> {
        let mut args = vec![
            self.config.cli.clone(),
            "--address".to_string(),
            self.config.containerd_addr.clone(),
        ];

        self.append_tls_flags(&mut args);

        args.push("pull".to_string());
        args.push(self.config.image.clone());

        args
    }

    /// Build the nerdctl run command arguments.
    pub fn build_run_command(&self) -> Vec<String> {
        self.build_run_command_with_extra_env(&HashMap::new())
    }

    /// Build the run command arguments with extra environment variables
    /// injected from workflow data, using the configured CLI binary.
    pub fn build_run_command_with_extra_env(
        &self,
        workflow_env: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut args = vec![
            self.config.cli.clone(),
            "--address".to_string(),
            self.config.containerd_addr.clone(),
        ];

        self.append_tls_flags(&mut args);

        args.push("run".to_string());
        args.push("--rm".to_string());

        args.push("--net".to_string());
        args.push(self.config.network.clone());

        args.push("--user".to_string());
        args.push(self.config.user.clone());

        if let Some(ref memory) = self.config.memory {
            args.push("--memory".to_string());
            args.push(memory.clone());
        }

        if let Some(ref cpu) = self.config.cpu {
            args.push("--cpus".to_string());
            args.push(cpu.clone());
        }

        if let Some(ref working_dir) = self.config.working_dir {
            args.push("-w".to_string());
            args.push(working_dir.clone());
        }

        // Inject workflow data env vars first, then config env vars (config overrides).
        for (key, value) in workflow_env {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }

        for (key, value) in &self.config.env {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }

        for vol in &self.config.volumes {
            args.push("-v".to_string());
            if vol.readonly {
                args.push(format!("{}:{}:ro", vol.source, vol.target));
            } else {
                args.push(format!("{}:{}", vol.source, vol.target));
            }
        }

        args.push(self.config.image.clone());

        // Add command or run
        if let Some(ref run) = self.config.run {
            args.push("sh".to_string());
            args.push("-c".to_string());
            args.push(run.clone());
        } else if let Some(ref command) = self.config.command {
            args.extend(command.iter().cloned());
        }

        args
    }

    /// Parse ##wfe[output key=value] lines from stdout.
    pub fn parse_outputs(stdout: &str) -> HashMap<String, String> {
        let mut outputs = HashMap::new();
        for line in stdout.lines() {
            if let Some(rest) = line.strip_prefix("##wfe[output ")
                && let Some(rest) = rest.strip_suffix(']')
                && let Some(eq_pos) = rest.find('=')
            {
                let name = rest[..eq_pos].trim().to_string();
                let value = rest[eq_pos + 1..].to_string();
                outputs.insert(name, value);
            }
        }
        outputs
    }

    /// Build the output data JSON value from step execution results.
    pub fn build_output_data(
        step_name: &str,
        stdout: &str,
        stderr: &str,
        exit_code: i32,
        parsed_outputs: &HashMap<String, String>,
    ) -> serde_json::Value {
        let mut outputs = serde_json::Map::new();
        for (key, value) in parsed_outputs {
            outputs.insert(key.clone(), serde_json::Value::String(value.clone()));
        }
        outputs.insert(
            format!("{step_name}.stdout"),
            serde_json::Value::String(stdout.to_string()),
        );
        outputs.insert(
            format!("{step_name}.stderr"),
            serde_json::Value::String(stderr.to_string()),
        );
        outputs.insert(
            format!("{step_name}.exit_code"),
            serde_json::Value::Number(serde_json::Number::from(exit_code)),
        );
        serde_json::Value::Object(outputs)
    }

    /// Build login commands for each configured registry auth entry.
    /// Returns a list of (registry, args) tuples.
    pub fn build_login_commands(&self) -> Vec<(String, Vec<String>)> {
        let mut commands = Vec::new();
        for (registry, auth) in &self.config.registry_auth {
            let mut args = vec![
                self.config.cli.clone(),
                "--address".to_string(),
                self.config.containerd_addr.clone(),
            ];
            self.append_tls_flags(&mut args);
            args.push("login".to_string());
            args.push("-u".to_string());
            args.push(auth.username.clone());
            args.push("-p".to_string());
            args.push(auth.password.clone());
            args.push(registry.clone());
            commands.push((registry.clone(), args));
        }
        commands
    }

    fn append_tls_flags(&self, args: &mut Vec<String>) {
        if let Some(ref ca) = self.config.tls.ca {
            args.push("--tlscacert".to_string());
            args.push(ca.clone());
        }
        if let Some(ref cert) = self.config.tls.cert {
            args.push("--tlscert".to_string());
            args.push(cert.clone());
        }
        if let Some(ref key) = self.config.tls.key {
            args.push("--tlskey".to_string());
            args.push(key.clone());
        }
    }

    /// Run registry login commands for each configured registry auth entry.
    async fn run_registry_logins(&self) -> Result<(), WfeError> {
        for (registry, auth) in &self.config.registry_auth {
            let mut cmd = tokio::process::Command::new(&self.config.cli);
            cmd.arg("--address")
                .arg(&self.config.containerd_addr);

            self.append_tls_flags_to_command(&mut cmd);

            cmd.arg("login")
                .arg("-u")
                .arg(&auth.username)
                .arg("-p")
                .arg(&auth.password)
                .arg(registry);

            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let cli = &self.config.cli;
            let output = cmd
                .output()
                .await
                .map_err(|e| WfeError::StepExecution(format!("Failed to run {cli} login for {registry}: {e}")))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(WfeError::StepExecution(format!(
                    "{cli} login failed for {registry}: {stderr}"
                )));
            }
        }
        Ok(())
    }

    fn append_tls_flags_to_command(&self, cmd: &mut tokio::process::Command) {
        if let Some(ref ca) = self.config.tls.ca {
            cmd.arg("--tlscacert").arg(ca);
        }
        if let Some(ref cert) = self.config.tls.cert {
            cmd.arg("--tlscert").arg(cert);
        }
        if let Some(ref key) = self.config.tls.key {
            cmd.arg("--tlskey").arg(key);
        }
    }
}

#[async_trait]
impl StepBody for ContainerdStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        // Run registry logins if configured.
        if !self.config.registry_auth.is_empty() {
            self.run_registry_logins().await?;
        }

        // Pull image if needed.
        let should_pull = match self.config.pull.as_str() {
            "always" => true,
            "never" => false,
            // "if-not-present" — always attempt pull, nerdctl handles caching
            _ => true,
        };

        if should_pull {
            let pull_args = self.build_pull_command();
            let mut pull_cmd = tokio::process::Command::new(&pull_args[0]);
            pull_cmd.args(&pull_args[1..]);
            pull_cmd.stdout(std::process::Stdio::piped());
            pull_cmd.stderr(std::process::Stdio::piped());

            let pull_output = pull_cmd
                .output()
                .await
                .map_err(|e| WfeError::StepExecution(format!("Failed to pull image: {e}")))?;

            if !pull_output.status.success() {
                let stderr = String::from_utf8_lossy(&pull_output.stderr);
                return Err(WfeError::StepExecution(format!(
                    "Image pull failed: {stderr}"
                )));
            }
        }

        // Build workflow env vars (same pattern as shell executor).
        let mut workflow_env = HashMap::new();
        if let Some(data_obj) = context.workflow.data.as_object() {
            for (key, value) in data_obj {
                let env_key = key.to_uppercase();
                let env_val = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                workflow_env.insert(env_key, env_val);
            }
        }

        // Build and spawn the run command.
        let run_args = self.build_run_command_with_extra_env(&workflow_env);
        let mut cmd = tokio::process::Command::new(&run_args[0]);
        cmd.args(&run_args[1..]);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Execute with optional timeout.
        let cli = &self.config.cli;
        let output = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, cmd.output()).await {
                Ok(result) => result.map_err(|e| {
                    WfeError::StepExecution(format!("Failed to spawn {cli} run: {e}"))
                })?,
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "Container execution timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            cmd.output()
                .await
                .map_err(|e| WfeError::StepExecution(format!("Failed to spawn {cli} run: {e}")))?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        if !output.status.success() {
            return Err(WfeError::StepExecution(format!(
                "Container exited with code {exit_code}\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        // Parse ##wfe[output key=value] lines from stdout.
        let parsed = Self::parse_outputs(&stdout);
        let step_name = context.step.name.as_deref().unwrap_or("unknown");
        let output_data = Self::build_output_data(step_name, &stdout, &stderr, exit_code, &parsed);

        Ok(ExecutionResult {
            proceed: true,
            output_data: Some(output_data),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{RegistryAuth, TlsConfig, VolumeMountConfig};
    use pretty_assertions::assert_eq;

    fn minimal_config() -> ContainerdConfig {
        ContainerdConfig {
            image: "alpine:3.18".to_string(),
            command: None,
            run: Some("echo hello".to_string()),
            env: HashMap::new(),
            volumes: vec![],
            working_dir: None,
            user: "65534:65534".to_string(),
            network: "none".to_string(),
            memory: None,
            cpu: None,
            pull: "if-not-present".to_string(),
            containerd_addr: "/run/containerd/containerd.sock".to_string(),
            cli: "nerdctl".to_string(),
            tls: TlsConfig::default(),
            registry_auth: HashMap::new(),
            timeout_ms: None,
        }
    }

    // ── build_run_command ───────────────────────────────────────────────

    #[test]
    fn build_run_command_minimal() {
        let step = ContainerdStep::new(minimal_config());
        let args = step.build_run_command();

        assert_eq!(args[0], "nerdctl");
        assert_eq!(args[1], "--address");
        assert_eq!(args[2], "/run/containerd/containerd.sock");
        assert_eq!(args[3], "run");
        assert_eq!(args[4], "--rm");
        assert_eq!(args[5], "--net");
        assert_eq!(args[6], "none");
        assert_eq!(args[7], "--user");
        assert_eq!(args[8], "65534:65534");
        assert_eq!(args[9], "alpine:3.18");
        assert_eq!(args[10], "sh");
        assert_eq!(args[11], "-c");
        assert_eq!(args[12], "echo hello");
    }

    #[test]
    fn build_run_command_with_command_array() {
        let mut config = minimal_config();
        config.run = None;
        config.command = Some(vec!["echo".to_string(), "hello".to_string(), "world".to_string()]);

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let len = args.len();
        assert_eq!(args[len - 3], "echo");
        assert_eq!(args[len - 2], "hello");
        assert_eq!(args[len - 1], "world");
        // Should NOT contain sh -c
        assert!(!args.contains(&"sh".to_string()) || args.iter().position(|a| a == "sh").is_none());
    }

    #[test]
    fn build_run_command_no_command_no_run() {
        let mut config = minimal_config();
        config.run = None;
        config.command = None;

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        // Should end with the image name, no trailing command
        assert_eq!(args.last().unwrap(), "alpine:3.18");
    }

    #[test]
    fn build_run_command_with_env() {
        let mut config = minimal_config();
        config.env = HashMap::from([
            ("FOO".to_string(), "bar".to_string()),
            ("BAZ".to_string(), "qux".to_string()),
        ]);

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let mut env_pairs: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-e" {
                env_pairs.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        }
        env_pairs.sort();
        assert!(env_pairs.contains(&"BAZ=qux".to_string()));
        assert!(env_pairs.contains(&"FOO=bar".to_string()));
    }

    #[test]
    fn build_run_command_with_volumes() {
        let mut config = minimal_config();
        config.volumes = vec![VolumeMountConfig {
            source: "/host/data".to_string(),
            target: "/container/data".to_string(),
            readonly: false,
        }];

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(args.contains(&"-v".to_string()));
        assert!(args.contains(&"/host/data:/container/data".to_string()));
    }

    #[test]
    fn build_run_command_with_volumes_readonly() {
        let mut config = minimal_config();
        config.volumes = vec![VolumeMountConfig {
            source: "/host/data".to_string(),
            target: "/container/data".to_string(),
            readonly: true,
        }];

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(args.contains(&"-v".to_string()));
        assert!(args.contains(&"/host/data:/container/data:ro".to_string()));
    }

    #[test]
    fn build_run_command_with_multiple_volumes() {
        let mut config = minimal_config();
        config.volumes = vec![
            VolumeMountConfig {
                source: "/a".to_string(),
                target: "/b".to_string(),
                readonly: false,
            },
            VolumeMountConfig {
                source: "/c".to_string(),
                target: "/d".to_string(),
                readonly: true,
            },
        ];

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(args.contains(&"/a:/b".to_string()));
        assert!(args.contains(&"/c:/d:ro".to_string()));
    }

    #[test]
    fn build_run_command_with_resource_limits() {
        let mut config = minimal_config();
        config.memory = Some("512m".to_string());
        config.cpu = Some("1.5".to_string());

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let mem_idx = args.iter().position(|a| a == "--memory").unwrap();
        assert_eq!(args[mem_idx + 1], "512m");

        let cpu_idx = args.iter().position(|a| a == "--cpus").unwrap();
        assert_eq!(args[cpu_idx + 1], "1.5");
    }

    #[test]
    fn build_run_command_memory_only() {
        let mut config = minimal_config();
        config.memory = Some("1g".to_string());

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(args.contains(&"--memory".to_string()));
        assert!(!args.contains(&"--cpus".to_string()));
    }

    #[test]
    fn build_run_command_cpu_only() {
        let mut config = minimal_config();
        config.cpu = Some("2.0".to_string());

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(args.contains(&"--cpus".to_string()));
        assert!(!args.contains(&"--memory".to_string()));
    }

    #[test]
    fn build_run_command_with_network_host() {
        let mut config = minimal_config();
        config.network = "host".to_string();

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let net_idx = args.iter().position(|a| a == "--net").unwrap();
        assert_eq!(args[net_idx + 1], "host");
    }

    #[test]
    fn build_run_command_with_network_bridge() {
        let mut config = minimal_config();
        config.network = "bridge".to_string();

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let net_idx = args.iter().position(|a| a == "--net").unwrap();
        assert_eq!(args[net_idx + 1], "bridge");
    }

    #[test]
    fn build_run_command_with_working_dir() {
        let mut config = minimal_config();
        config.working_dir = Some("/app".to_string());

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let w_idx = args.iter().position(|a| a == "-w").unwrap();
        assert_eq!(args[w_idx + 1], "/app");
    }

    #[test]
    fn build_run_command_without_working_dir() {
        let config = minimal_config();
        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(!args.contains(&"-w".to_string()));
    }

    #[test]
    fn build_run_command_with_user() {
        let mut config = minimal_config();
        config.user = "1000:1000".to_string();

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let user_idx = args.iter().position(|a| a == "--user").unwrap();
        assert_eq!(args[user_idx + 1], "1000:1000");
    }

    #[test]
    fn build_run_command_root_user() {
        let mut config = minimal_config();
        config.user = "0:0".to_string();

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let user_idx = args.iter().position(|a| a == "--user").unwrap();
        assert_eq!(args[user_idx + 1], "0:0");
    }

    #[test]
    fn build_run_command_with_tls() {
        let mut config = minimal_config();
        config.tls = TlsConfig {
            ca: Some("/ca.pem".to_string()),
            cert: Some("/cert.pem".to_string()),
            key: Some("/key.pem".to_string()),
        };

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        let ca_idx = args.iter().position(|a| a == "--tlscacert").unwrap();
        assert_eq!(args[ca_idx + 1], "/ca.pem");

        let cert_idx = args.iter().position(|a| a == "--tlscert").unwrap();
        assert_eq!(args[cert_idx + 1], "/cert.pem");

        let key_idx = args.iter().position(|a| a == "--tlskey").unwrap();
        assert_eq!(args[key_idx + 1], "/key.pem");
    }

    #[test]
    fn build_run_command_with_partial_tls() {
        let mut config = minimal_config();
        config.tls = TlsConfig {
            ca: Some("/ca.pem".to_string()),
            cert: None,
            key: None,
        };

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(args.contains(&"--tlscacert".to_string()));
        assert!(!args.contains(&"--tlscert".to_string()));
        assert!(!args.contains(&"--tlskey".to_string()));
    }

    #[test]
    fn build_run_command_no_tls() {
        let config = minimal_config();
        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert!(!args.contains(&"--tlscacert".to_string()));
        assert!(!args.contains(&"--tlscert".to_string()));
        assert!(!args.contains(&"--tlskey".to_string()));
    }

    #[test]
    fn build_run_command_with_custom_address() {
        let mut config = minimal_config();
        config.containerd_addr = "/custom/sock".to_string();

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        assert_eq!(args[1], "--address");
        assert_eq!(args[2], "/custom/sock");
    }

    #[test]
    fn build_run_command_with_all_flags() {
        let mut config = minimal_config();
        config.network = "host".to_string();
        config.user = "1000:1000".to_string();
        config.memory = Some("256m".to_string());
        config.cpu = Some("0.5".to_string());
        config.working_dir = Some("/workspace".to_string());
        config.env = HashMap::from([("KEY".to_string(), "val".to_string())]);
        config.volumes = vec![VolumeMountConfig {
            source: "/src".to_string(),
            target: "/dst".to_string(),
            readonly: true,
        }];
        config.tls = TlsConfig {
            ca: Some("/ca.pem".to_string()),
            cert: Some("/cert.pem".to_string()),
            key: Some("/key.pem".to_string()),
        };

        let step = ContainerdStep::new(config);
        let args = step.build_run_command();

        // Verify all flags are present
        assert!(args.contains(&"--net".to_string()));
        assert!(args.contains(&"host".to_string()));
        assert!(args.contains(&"--user".to_string()));
        assert!(args.contains(&"1000:1000".to_string()));
        assert!(args.contains(&"--memory".to_string()));
        assert!(args.contains(&"256m".to_string()));
        assert!(args.contains(&"--cpus".to_string()));
        assert!(args.contains(&"0.5".to_string()));
        assert!(args.contains(&"-w".to_string()));
        assert!(args.contains(&"/workspace".to_string()));
        assert!(args.contains(&"-e".to_string()));
        assert!(args.contains(&"KEY=val".to_string()));
        assert!(args.contains(&"-v".to_string()));
        assert!(args.contains(&"/src:/dst:ro".to_string()));
        assert!(args.contains(&"--tlscacert".to_string()));
        assert!(args.contains(&"--tlscert".to_string()));
        assert!(args.contains(&"--tlskey".to_string()));
    }

    // ── build_run_command_with_extra_env ────────────────────────────────

    #[test]
    fn build_run_command_with_extra_env_merges() {
        let mut config = minimal_config();
        config.env = HashMap::from([("CONFIG_VAR".to_string(), "from_config".to_string())]);

        let step = ContainerdStep::new(config);
        let workflow_env = HashMap::from([("WF_VAR".to_string(), "from_workflow".to_string())]);
        let args = step.build_run_command_with_extra_env(&workflow_env);

        let mut env_pairs: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-e" {
                env_pairs.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        }

        assert!(env_pairs.contains(&"CONFIG_VAR=from_config".to_string()));
        assert!(env_pairs.contains(&"WF_VAR=from_workflow".to_string()));
    }

    #[test]
    fn build_run_command_with_extra_env_empty() {
        let config = minimal_config();
        let step = ContainerdStep::new(config);
        let args_no_extra = step.build_run_command_with_extra_env(&HashMap::new());
        let args_default = step.build_run_command();

        assert_eq!(args_no_extra, args_default);
    }

    // ── build_pull_command ──────────────────────────────────────────────

    #[test]
    fn build_pull_command_basic() {
        let step = ContainerdStep::new(minimal_config());
        let args = step.build_pull_command();

        assert_eq!(args[0], "nerdctl");
        assert_eq!(args[1], "--address");
        assert_eq!(args[2], "/run/containerd/containerd.sock");
        assert_eq!(args[3], "pull");
        assert_eq!(args[4], "alpine:3.18");
    }

    #[test]
    fn build_pull_command_with_tls() {
        let mut config = minimal_config();
        config.tls = TlsConfig {
            ca: Some("/ca.pem".to_string()),
            cert: None,
            key: None,
        };

        let step = ContainerdStep::new(config);
        let args = step.build_pull_command();

        let ca_idx = args.iter().position(|a| a == "--tlscacert").unwrap();
        assert_eq!(args[ca_idx + 1], "/ca.pem");
        assert!(args.iter().position(|a| a == "--tlscert").is_none());
        assert!(args.iter().position(|a| a == "--tlskey").is_none());
    }

    #[test]
    fn build_pull_command_with_full_tls() {
        let mut config = minimal_config();
        config.tls = TlsConfig {
            ca: Some("/ca.pem".to_string()),
            cert: Some("/cert.pem".to_string()),
            key: Some("/key.pem".to_string()),
        };

        let step = ContainerdStep::new(config);
        let args = step.build_pull_command();

        assert!(args.contains(&"--tlscacert".to_string()));
        assert!(args.contains(&"--tlscert".to_string()));
        assert!(args.contains(&"--tlskey".to_string()));
        // pull should still be present after tls flags
        assert!(args.contains(&"pull".to_string()));
    }

    #[test]
    fn build_pull_command_custom_address() {
        let mut config = minimal_config();
        config.containerd_addr = "/var/run/custom.sock".to_string();

        let step = ContainerdStep::new(config);
        let args = step.build_pull_command();

        assert_eq!(args[2], "/var/run/custom.sock");
    }

    // ── parse_outputs ──────────────────────────────────────────────────

    #[test]
    fn parse_outputs_empty() {
        let outputs = ContainerdStep::parse_outputs("");
        assert!(outputs.is_empty());
    }

    #[test]
    fn parse_outputs_single() {
        let stdout = "some log line\n##wfe[output version=1.2.3]\nmore logs\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs.get("version").unwrap(), "1.2.3");
    }

    #[test]
    fn parse_outputs_multiple() {
        let stdout = "##wfe[output foo=bar]\n##wfe[output baz=qux]\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs.get("foo").unwrap(), "bar");
        assert_eq!(outputs.get("baz").unwrap(), "qux");
    }

    #[test]
    fn parse_outputs_mixed_with_regular_stdout() {
        let stdout = "Starting container...\n\
                      Pulling image...\n\
                      ##wfe[output digest=sha256:abc123]\n\
                      Running tests...\n\
                      ##wfe[output result=pass]\n\
                      Done.\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs.get("digest").unwrap(), "sha256:abc123");
        assert_eq!(outputs.get("result").unwrap(), "pass");
    }

    #[test]
    fn parse_outputs_no_wfe_lines() {
        let stdout = "line 1\nline 2\nline 3\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert!(outputs.is_empty());
    }

    #[test]
    fn parse_outputs_value_with_equals_sign() {
        let stdout = "##wfe[output url=https://example.com?a=1&b=2]\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert_eq!(outputs.len(), 1);
        assert_eq!(
            outputs.get("url").unwrap(),
            "https://example.com?a=1&b=2"
        );
    }

    #[test]
    fn parse_outputs_ignores_malformed_lines() {
        let stdout = "##wfe[output ]\n\
                      ##wfe[output no_equals]\n\
                      ##wfe[output valid=yes]\n\
                      ##wfe[output_extra bad=val]\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs.get("valid").unwrap(), "yes");
    }

    #[test]
    fn parse_outputs_overwrites_duplicate_keys() {
        let stdout = "##wfe[output key=first]\n##wfe[output key=second]\n";
        let outputs = ContainerdStep::parse_outputs(stdout);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs.get("key").unwrap(), "second");
    }

    // ── build_output_data ──────────────────────────────────────────────

    #[test]
    fn build_output_data_basic() {
        let parsed = HashMap::from([("result".to_string(), "success".to_string())]);
        let data = ContainerdStep::build_output_data(
            "my_step",
            "hello world\n",
            "",
            0,
            &parsed,
        );

        let obj = data.as_object().unwrap();
        assert_eq!(obj.get("result").unwrap(), "success");
        assert_eq!(obj.get("my_step.stdout").unwrap(), "hello world\n");
        assert_eq!(obj.get("my_step.stderr").unwrap(), "");
        assert_eq!(obj.get("my_step.exit_code").unwrap(), 0);
    }

    #[test]
    fn build_output_data_no_parsed_outputs() {
        let data = ContainerdStep::build_output_data(
            "step1",
            "out",
            "err",
            1,
            &HashMap::new(),
        );

        let obj = data.as_object().unwrap();
        assert_eq!(obj.len(), 3); // stdout, stderr, exit_code
        assert_eq!(obj.get("step1.stdout").unwrap(), "out");
        assert_eq!(obj.get("step1.stderr").unwrap(), "err");
        assert_eq!(obj.get("step1.exit_code").unwrap(), 1);
    }

    #[test]
    fn build_output_data_with_multiple_parsed_outputs() {
        let parsed = HashMap::from([
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
            ("c".to_string(), "3".to_string()),
        ]);
        let data = ContainerdStep::build_output_data("s", "", "", 0, &parsed);

        let obj = data.as_object().unwrap();
        assert_eq!(obj.get("a").unwrap(), "1");
        assert_eq!(obj.get("b").unwrap(), "2");
        assert_eq!(obj.get("c").unwrap(), "3");
        // Plus the 3 standard keys
        assert_eq!(obj.len(), 6);
    }

    #[test]
    fn build_output_data_negative_exit_code() {
        let data = ContainerdStep::build_output_data("s", "", "", -1, &HashMap::new());
        let obj = data.as_object().unwrap();
        assert_eq!(obj.get("s.exit_code").unwrap(), -1);
    }

    // ── build_login_commands ───────────────────────────────────────────

    #[test]
    fn build_login_commands_empty_registry_auth() {
        let config = minimal_config();
        let step = ContainerdStep::new(config);
        let commands = step.build_login_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn build_login_commands_single_registry() {
        let mut config = minimal_config();
        config.registry_auth = HashMap::from([(
            "registry.example.com".to_string(),
            RegistryAuth {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
        )]);

        let step = ContainerdStep::new(config);
        let commands = step.build_login_commands();

        assert_eq!(commands.len(), 1);
        let (registry, args) = &commands[0];
        assert_eq!(registry, "registry.example.com");
        assert_eq!(args[0], "nerdctl");
        assert_eq!(args[1], "--address");
        assert_eq!(args[2], "/run/containerd/containerd.sock");
        assert!(args.contains(&"login".to_string()));
        assert!(args.contains(&"-u".to_string()));
        assert!(args.contains(&"user".to_string()));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"pass".to_string()));
        assert!(args.contains(&"registry.example.com".to_string()));
    }

    #[test]
    fn build_login_commands_multiple_registries() {
        let mut config = minimal_config();
        config.registry_auth = HashMap::from([
            (
                "reg1.io".to_string(),
                RegistryAuth {
                    username: "u1".to_string(),
                    password: "p1".to_string(),
                },
            ),
            (
                "reg2.io".to_string(),
                RegistryAuth {
                    username: "u2".to_string(),
                    password: "p2".to_string(),
                },
            ),
        ]);

        let step = ContainerdStep::new(config);
        let commands = step.build_login_commands();

        assert_eq!(commands.len(), 2);
        let registries: Vec<&String> = commands.iter().map(|(r, _)| r).collect();
        assert!(registries.contains(&&"reg1.io".to_string()));
        assert!(registries.contains(&&"reg2.io".to_string()));
    }

    #[test]
    fn build_login_commands_with_tls() {
        let mut config = minimal_config();
        config.tls = TlsConfig {
            ca: Some("/ca.pem".to_string()),
            cert: Some("/cert.pem".to_string()),
            key: None,
        };
        config.registry_auth = HashMap::from([(
            "secure.io".to_string(),
            RegistryAuth {
                username: "admin".to_string(),
                password: "secret".to_string(),
            },
        )]);

        let step = ContainerdStep::new(config);
        let commands = step.build_login_commands();

        assert_eq!(commands.len(), 1);
        let (_, args) = &commands[0];
        assert!(args.contains(&"--tlscacert".to_string()));
        assert!(args.contains(&"--tlscert".to_string()));
        assert!(!args.contains(&"--tlskey".to_string()));
    }

    // ── ContainerdStep::new ────────────────────────────────────────────

    #[test]
    fn new_creates_step_with_config() {
        let config = minimal_config();
        let step = ContainerdStep::new(config.clone());
        // Verify it stored the config by checking a generated command
        let args = step.build_run_command();
        assert!(args.contains(&config.image));
    }
}
