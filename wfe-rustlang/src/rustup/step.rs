use async_trait::async_trait;
use wfe_core::WfeError;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

use crate::rustup::config::{RustupCommand, RustupConfig};

pub struct RustupStep {
    config: RustupConfig,
}

impl RustupStep {
    pub fn new(config: RustupConfig) -> Self {
        Self { config }
    }

    pub fn build_command(&self) -> tokio::process::Command {
        match self.config.command {
            RustupCommand::Install => self.build_install_command(),
            RustupCommand::ToolchainInstall => self.build_toolchain_install_command(),
            RustupCommand::ComponentAdd => self.build_component_add_command(),
            RustupCommand::TargetAdd => self.build_target_add_command(),
        }
    }

    fn build_install_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("sh");
        // Pipe rustup-init through sh with non-interactive flag.
        let mut script =
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y".to_string();

        if let Some(ref profile) = self.config.profile {
            script.push_str(&format!(" --profile {profile}"));
        }

        if let Some(ref tc) = self.config.default_toolchain {
            script.push_str(&format!(" --default-toolchain {tc}"));
        }

        for arg in &self.config.extra_args {
            script.push_str(&format!(" {arg}"));
        }

        cmd.arg("-c").arg(&script);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd
    }

    fn build_toolchain_install_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("rustup");
        cmd.args(["toolchain", "install"]);

        if let Some(ref tc) = self.config.toolchain {
            cmd.arg(tc);
        }

        if let Some(ref profile) = self.config.profile {
            cmd.args(["--profile", profile]);
        }

        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd
    }

    fn build_component_add_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("rustup");
        cmd.args(["component", "add"]);

        for component in &self.config.components {
            cmd.arg(component);
        }

        if let Some(ref tc) = self.config.toolchain {
            cmd.args(["--toolchain", tc]);
        }

        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd
    }

    fn build_target_add_command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("rustup");
        cmd.args(["target", "add"]);

        for target in &self.config.targets {
            cmd.arg(target);
        }

        if let Some(ref tc) = self.config.toolchain {
            cmd.args(["--toolchain", tc]);
        }

        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd
    }
}

#[async_trait]
impl StepBody for RustupStep {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let step_name = context.step.name.as_deref().unwrap_or("unknown");
        let subcmd = self.config.command.as_str();

        tracing::info!(step = step_name, command = subcmd, "running rustup");

        let mut cmd = self.build_command();

        let output = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, cmd.output()).await {
                Ok(result) => result.map_err(|e| {
                    WfeError::StepExecution(format!("Failed to spawn rustup {subcmd}: {e}"))
                })?,
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "rustup {subcmd} timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            cmd.output().await.map_err(|e| {
                WfeError::StepExecution(format!("Failed to spawn rustup {subcmd}: {e}"))
            })?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(WfeError::StepExecution(format!(
                "rustup {subcmd} exited with code {code}\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        let mut outputs = serde_json::Map::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn install_config() -> RustupConfig {
        RustupConfig {
            command: RustupCommand::Install,
            toolchain: None,
            components: vec![],
            targets: vec![],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        }
    }

    #[test]
    fn build_install_command_minimal() {
        let step = RustupStep::new(install_config());
        let cmd = step.build_command();
        let prog = cmd.as_std().get_program().to_str().unwrap();
        assert_eq!(prog, "sh");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args[0], "-c");
        assert!(args[1].contains("rustup.rs"));
        assert!(args[1].contains("-y"));
    }

    #[test]
    fn build_install_command_with_profile_and_toolchain() {
        let mut config = install_config();
        config.profile = Some("minimal".to_string());
        config.default_toolchain = Some("nightly".to_string());
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert!(args[1].contains("--profile minimal"));
        assert!(args[1].contains("--default-toolchain nightly"));
    }

    #[test]
    fn build_install_command_with_extra_args() {
        let mut config = install_config();
        config.extra_args = vec!["--no-modify-path".to_string()];
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert!(args[1].contains("--no-modify-path"));
    }

    #[test]
    fn build_toolchain_install_command() {
        let config = RustupConfig {
            command: RustupCommand::ToolchainInstall,
            toolchain: Some("nightly-2024-06-01".to_string()),
            components: vec![],
            targets: vec![],
            profile: Some("minimal".to_string()),
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let prog = cmd.as_std().get_program().to_str().unwrap();
        assert_eq!(prog, "rustup");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "toolchain",
                "install",
                "nightly-2024-06-01",
                "--profile",
                "minimal"
            ]
        );
    }

    #[test]
    fn build_toolchain_install_with_extra_args() {
        let config = RustupConfig {
            command: RustupCommand::ToolchainInstall,
            toolchain: Some("stable".to_string()),
            components: vec![],
            targets: vec![],
            profile: None,
            default_toolchain: None,
            extra_args: vec!["--force".to_string()],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["toolchain", "install", "stable", "--force"]);
    }

    #[test]
    fn build_component_add_command() {
        let config = RustupConfig {
            command: RustupCommand::ComponentAdd,
            toolchain: Some("nightly".to_string()),
            components: vec!["clippy".to_string(), "rustfmt".to_string()],
            targets: vec![],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let prog = cmd.as_std().get_program().to_str().unwrap();
        assert_eq!(prog, "rustup");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "component",
                "add",
                "clippy",
                "rustfmt",
                "--toolchain",
                "nightly"
            ]
        );
    }

    #[test]
    fn build_component_add_without_toolchain() {
        let config = RustupConfig {
            command: RustupCommand::ComponentAdd,
            toolchain: None,
            components: vec!["rust-src".to_string()],
            targets: vec![],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["component", "add", "rust-src"]);
    }

    #[test]
    fn build_target_add_command() {
        let config = RustupConfig {
            command: RustupCommand::TargetAdd,
            toolchain: Some("stable".to_string()),
            components: vec![],
            targets: vec!["wasm32-unknown-unknown".to_string()],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let prog = cmd.as_std().get_program().to_str().unwrap();
        assert_eq!(prog, "rustup");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "target",
                "add",
                "wasm32-unknown-unknown",
                "--toolchain",
                "stable"
            ]
        );
    }

    #[test]
    fn build_target_add_multiple_targets() {
        let config = RustupConfig {
            command: RustupCommand::TargetAdd,
            toolchain: None,
            components: vec![],
            targets: vec![
                "wasm32-unknown-unknown".to_string(),
                "aarch64-linux-android".to_string(),
            ],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "target",
                "add",
                "wasm32-unknown-unknown",
                "aarch64-linux-android"
            ]
        );
    }

    #[test]
    fn build_target_add_with_extra_args() {
        let config = RustupConfig {
            command: RustupCommand::TargetAdd,
            toolchain: Some("nightly".to_string()),
            components: vec![],
            targets: vec!["x86_64-unknown-linux-musl".to_string()],
            profile: None,
            default_toolchain: None,
            extra_args: vec!["--force".to_string()],
            timeout_ms: None,
        };
        let step = RustupStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "target",
                "add",
                "x86_64-unknown-linux-musl",
                "--toolchain",
                "nightly",
                "--force"
            ]
        );
    }
}
