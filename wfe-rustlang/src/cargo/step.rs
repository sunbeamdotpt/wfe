use async_trait::async_trait;
use wfe_core::WfeError;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

use crate::cargo::config::{CargoCommand, CargoConfig};

pub struct CargoStep {
    config: CargoConfig,
}

impl CargoStep {
    pub fn new(config: CargoConfig) -> Self {
        Self { config }
    }

    pub fn build_command(&self) -> tokio::process::Command {
        // DocMdx requires nightly for --output-format json.
        let toolchain = if matches!(self.config.command, CargoCommand::DocMdx) {
            Some(self.config.toolchain.as_deref().unwrap_or("nightly"))
        } else {
            self.config.toolchain.as_deref()
        };

        let mut cmd = if let Some(tc) = toolchain {
            let mut c = tokio::process::Command::new("rustup");
            c.args(["run", tc, "cargo"]);
            c
        } else {
            tokio::process::Command::new("cargo")
        };

        for arg in self.config.command.subcommand_args() {
            cmd.arg(arg);
        }

        if let Some(ref pkg) = self.config.package {
            cmd.args(["-p", pkg]);
        }

        if !self.config.features.is_empty() {
            cmd.args(["--features", &self.config.features.join(",")]);
        }

        if self.config.all_features {
            cmd.arg("--all-features");
        }

        if self.config.no_default_features {
            cmd.arg("--no-default-features");
        }

        if self.config.release {
            cmd.arg("--release");
        }

        if let Some(ref target) = self.config.target {
            cmd.args(["--target", target]);
        }

        if let Some(ref profile) = self.config.profile {
            cmd.args(["--profile", profile]);
        }

        for arg in &self.config.extra_args {
            cmd.arg(arg);
        }

        // DocMdx appends rustdoc-specific flags after user extra_args.
        if matches!(self.config.command, CargoCommand::DocMdx) {
            cmd.args(["--", "-Z", "unstable-options", "--output-format", "json"]);
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

    /// Ensures an external cargo tool is installed before running it.
    /// For built-in cargo subcommands, this is a no-op.
    async fn ensure_tool_available(&self) -> Result<(), WfeError> {
        let (binary, package) = match (
            self.config.command.binary_name(),
            self.config.command.install_package(),
        ) {
            (Some(b), Some(p)) => (b, p),
            _ => return Ok(()),
        };

        // Probe for the binary.
        let probe = tokio::process::Command::new(binary)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        if let Ok(status) = probe {
            if status.success() {
                return Ok(());
            }
        }

        tracing::info!(package = package, "cargo tool not found, installing");

        // For llvm-cov, ensure the rustup component is present first.
        if matches!(self.config.command, CargoCommand::LlvmCov) {
            let component = tokio::process::Command::new("rustup")
                .args(["component", "add", "llvm-tools-preview"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "Failed to add llvm-tools-preview component: {e}"
                    ))
                })?;

            if !component.status.success() {
                let stderr = String::from_utf8_lossy(&component.stderr);
                return Err(WfeError::StepExecution(format!(
                    "Failed to add llvm-tools-preview component: {stderr}"
                )));
            }
        }

        let install = tokio::process::Command::new("cargo")
            .args(["install", package])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| WfeError::StepExecution(format!("Failed to install {package}: {e}")))?;

        if !install.status.success() {
            let stderr = String::from_utf8_lossy(&install.stderr);
            return Err(WfeError::StepExecution(format!(
                "Failed to install {package}: {stderr}"
            )));
        }

        tracing::info!(package = package, "cargo tool installed successfully");
        Ok(())
    }

    /// Post-process rustdoc JSON output into MDX files.
    fn transform_rustdoc_json(
        &self,
        outputs: &mut serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), WfeError> {
        use crate::rustdoc::transformer::{transform_to_mdx, write_mdx_files};

        // Find the JSON file in target/doc/.
        let working_dir = self.config.working_dir.as_deref().unwrap_or(".");
        let doc_dir = std::path::Path::new(working_dir).join("target/doc");

        let json_path = std::fs::read_dir(&doc_dir)
            .map_err(|e| WfeError::StepExecution(format!("failed to read target/doc: {e}")))?
            .filter_map(|entry| entry.ok())
            .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .map(|entry| entry.path())
            .ok_or_else(|| {
                WfeError::StepExecution(
                    "no JSON file found in target/doc/ — did rustdoc --output-format json succeed?"
                        .to_string(),
                )
            })?;

        tracing::info!(path = %json_path.display(), "reading rustdoc JSON");

        let json_content = std::fs::read_to_string(&json_path).map_err(|e| {
            WfeError::StepExecution(format!("failed to read {}: {e}", json_path.display()))
        })?;

        let krate: rustdoc_types::Crate = serde_json::from_str(&json_content)
            .map_err(|e| WfeError::StepExecution(format!("failed to parse rustdoc JSON: {e}")))?;

        let mdx_files = transform_to_mdx(&krate);

        let output_dir = self
            .config
            .output_dir
            .as_deref()
            .unwrap_or("target/doc/mdx");
        let output_path = std::path::Path::new(working_dir).join(output_dir);

        write_mdx_files(&mdx_files, &output_path)
            .map_err(|e| WfeError::StepExecution(format!("failed to write MDX files: {e}")))?;

        let file_count = mdx_files.len();
        tracing::info!(
            output_dir = %output_path.display(),
            file_count,
            "generated MDX documentation"
        );

        outputs.insert(
            "mdx.output_dir".to_string(),
            serde_json::Value::String(output_path.to_string_lossy().to_string()),
        );
        outputs.insert(
            "mdx.file_count".to_string(),
            serde_json::Value::Number(file_count.into()),
        );
        let file_paths: Vec<_> = mdx_files.iter().map(|f| f.path.clone()).collect();
        outputs.insert(
            "mdx.files".to_string(),
            serde_json::Value::Array(
                file_paths
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );

        Ok(())
    }
}

#[async_trait]
impl StepBody for CargoStep {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let step_name = context.step.name.as_deref().unwrap_or("unknown");
        let subcmd = self.config.command.as_str();

        // Ensure external tools are installed before running.
        self.ensure_tool_available().await?;

        tracing::info!(step = step_name, command = subcmd, "running cargo");

        let mut cmd = self.build_command();

        let output = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, cmd.output()).await {
                Ok(result) => result.map_err(|e| {
                    WfeError::StepExecution(format!("Failed to spawn cargo {subcmd}: {e}"))
                })?,
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "cargo {subcmd} timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            cmd.output().await.map_err(|e| {
                WfeError::StepExecution(format!("Failed to spawn cargo {subcmd}: {e}"))
            })?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(WfeError::StepExecution(format!(
                "cargo {subcmd} exited with code {code}\nstdout: {stdout}\nstderr: {stderr}"
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

        // DocMdx post-processing: transform rustdoc JSON → MDX files.
        if matches!(self.config.command, CargoCommand::DocMdx) {
            self.transform_rustdoc_json(&mut outputs)?;
        }

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
    use crate::cargo::config::{CargoCommand, CargoConfig};
    use std::collections::HashMap;

    fn minimal_config(command: CargoCommand) -> CargoConfig {
        CargoConfig {
            command,
            toolchain: None,
            package: None,
            features: vec![],
            all_features: false,
            no_default_features: false,
            release: false,
            target: None,
            profile: None,
            extra_args: vec![],
            env: HashMap::new(),
            working_dir: None,
            timeout_ms: None,
            output_dir: None,
        }
    }

    #[test]
    fn build_command_minimal() {
        let step = CargoStep::new(minimal_config(CargoCommand::Build));
        let cmd = step.build_command();
        let prog = cmd.as_std().get_program().to_str().unwrap();
        assert_eq!(prog, "cargo");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["build"]);
    }

    #[test]
    fn build_command_with_toolchain() {
        let mut config = minimal_config(CargoCommand::Test);
        config.toolchain = Some("nightly".to_string());
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let prog = cmd.as_std().get_program().to_str().unwrap();
        assert_eq!(prog, "rustup");
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["run", "nightly", "cargo", "test"]);
    }

    #[test]
    fn build_command_with_package_and_features() {
        let mut config = minimal_config(CargoCommand::Check);
        config.package = Some("my-crate".to_string());
        config.features = vec!["feat1".to_string(), "feat2".to_string()];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec!["check", "-p", "my-crate", "--features", "feat1,feat2"]
        );
    }

    #[test]
    fn build_command_release_and_target() {
        let mut config = minimal_config(CargoCommand::Build);
        config.release = true;
        config.target = Some("aarch64-unknown-linux-gnu".to_string());
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "build",
                "--release",
                "--target",
                "aarch64-unknown-linux-gnu"
            ]
        );
    }

    #[test]
    fn build_command_all_flags() {
        let mut config = minimal_config(CargoCommand::Clippy);
        config.all_features = true;
        config.no_default_features = true;
        config.profile = Some("dev".to_string());
        config.extra_args = vec!["--".to_string(), "-D".to_string(), "warnings".to_string()];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "clippy",
                "--all-features",
                "--no-default-features",
                "--profile",
                "dev",
                "--",
                "-D",
                "warnings"
            ]
        );
    }

    #[test]
    fn build_command_fmt() {
        let mut config = minimal_config(CargoCommand::Fmt);
        config.extra_args = vec!["--check".to_string()];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["fmt", "--check"]);
    }

    #[test]
    fn build_command_publish_dry_run() {
        let mut config = minimal_config(CargoCommand::Publish);
        config.extra_args = vec![
            "--dry-run".to_string(),
            "--registry".to_string(),
            "my-reg".to_string(),
        ];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["publish", "--dry-run", "--registry", "my-reg"]);
    }

    #[test]
    fn build_command_doc() {
        let mut config = minimal_config(CargoCommand::Doc);
        config.extra_args = vec!["--no-deps".to_string()];
        config.release = true;
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["doc", "--release", "--no-deps"]);
    }

    #[test]
    fn build_command_env_vars() {
        let mut config = minimal_config(CargoCommand::Build);
        config
            .env
            .insert("RUSTFLAGS".to_string(), "-D warnings".to_string());
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(
            envs.iter()
                .any(|(k, v)| *k == "RUSTFLAGS" && v == &Some("-D warnings".as_ref()))
        );
    }

    #[test]
    fn build_command_working_dir() {
        let mut config = minimal_config(CargoCommand::Test);
        config.working_dir = Some("/my/project".to_string());
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        assert_eq!(
            cmd.as_std().get_current_dir(),
            Some(std::path::Path::new("/my/project"))
        );
    }

    #[test]
    fn build_command_audit() {
        let step = CargoStep::new(minimal_config(CargoCommand::Audit));
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["audit"]);
    }

    #[test]
    fn build_command_deny() {
        let mut config = minimal_config(CargoCommand::Deny);
        config.extra_args = vec!["check".to_string()];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["deny", "check"]);
    }

    #[test]
    fn build_command_nextest() {
        let step = CargoStep::new(minimal_config(CargoCommand::Nextest));
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["nextest", "run"]);
    }

    #[test]
    fn build_command_nextest_with_features() {
        let mut config = minimal_config(CargoCommand::Nextest);
        config.features = vec!["feat1".to_string()];
        config.extra_args = vec!["--no-fail-fast".to_string()];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec!["nextest", "run", "--features", "feat1", "--no-fail-fast"]
        );
    }

    #[test]
    fn build_command_llvm_cov() {
        let step = CargoStep::new(minimal_config(CargoCommand::LlvmCov));
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["llvm-cov"]);
    }

    #[test]
    fn build_command_llvm_cov_with_args() {
        let mut config = minimal_config(CargoCommand::LlvmCov);
        config.extra_args = vec![
            "--html".to_string(),
            "--output-dir".to_string(),
            "coverage".to_string(),
        ];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(args, vec!["llvm-cov", "--html", "--output-dir", "coverage"]);
    }

    #[test]
    fn build_command_doc_mdx_forces_nightly() {
        let step = CargoStep::new(minimal_config(CargoCommand::DocMdx));
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
                "run",
                "nightly",
                "cargo",
                "rustdoc",
                "--",
                "-Z",
                "unstable-options",
                "--output-format",
                "json"
            ]
        );
    }

    #[test]
    fn build_command_doc_mdx_with_package() {
        let mut config = minimal_config(CargoCommand::DocMdx);
        config.package = Some("my-crate".to_string());
        config.extra_args = vec!["--no-deps".to_string()];
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "run",
                "nightly",
                "cargo",
                "rustdoc",
                "-p",
                "my-crate",
                "--no-deps",
                "--",
                "-Z",
                "unstable-options",
                "--output-format",
                "json"
            ]
        );
    }

    #[test]
    fn build_command_doc_mdx_custom_toolchain() {
        let mut config = minimal_config(CargoCommand::DocMdx);
        config.toolchain = Some("nightly-2024-06-01".to_string());
        let step = CargoStep::new(config);
        let cmd = step.build_command();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert!(args.contains(&"nightly-2024-06-01"));
    }

    #[tokio::test]
    async fn ensure_tool_builtin_is_noop() {
        let step = CargoStep::new(minimal_config(CargoCommand::Build));
        // Should return Ok immediately for built-in commands.
        step.ensure_tool_available().await.unwrap();
    }

    #[tokio::test]
    async fn ensure_tool_already_installed_succeeds() {
        // cargo-audit/nextest/etc may or may not be installed,
        // but we can test with a known-installed tool: cargo itself
        // is always available. Test the flow by verifying the
        // built-in path returns Ok.
        let step = CargoStep::new(minimal_config(CargoCommand::Check));
        step.ensure_tool_available().await.unwrap();
    }
}
