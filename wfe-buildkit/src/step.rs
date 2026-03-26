use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use regex::Regex;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};
use wfe_core::WfeError;

use crate::config::BuildkitConfig;

/// A workflow step that builds container images via the `buildctl` CLI.
pub struct BuildkitStep {
    config: BuildkitConfig,
}

impl BuildkitStep {
    /// Create a new BuildKit step from configuration.
    pub fn new(config: BuildkitConfig) -> Self {
        Self { config }
    }

    /// Build the `buildctl` command arguments without executing.
    ///
    /// Returns the full argument list starting with "buildctl". Useful for
    /// testing and debugging without a running BuildKit daemon.
    pub fn build_command(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        args.push("buildctl".to_string());
        args.push("--addr".to_string());
        args.push(self.config.buildkit_addr.clone());

        // TLS flags
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

        args.push("build".to_string());
        args.push("--frontend".to_string());
        args.push("dockerfile.v0".to_string());

        // Context directory
        args.push("--local".to_string());
        args.push(format!("context={}", self.config.context));

        // Dockerfile directory (parent of the Dockerfile path)
        let dockerfile_dir = Path::new(&self.config.dockerfile)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        let dockerfile_dir = if dockerfile_dir.is_empty() {
            ".".to_string()
        } else {
            dockerfile_dir
        };
        args.push("--local".to_string());
        args.push(format!("dockerfile={dockerfile_dir}"));

        // Dockerfile filename override (if not just "Dockerfile")
        let dockerfile_name = Path::new(&self.config.dockerfile)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "Dockerfile".to_string());
        if dockerfile_name != "Dockerfile" {
            args.push("--opt".to_string());
            args.push(format!("filename={dockerfile_name}"));
        }

        // Target
        if let Some(ref target) = self.config.target {
            args.push("--opt".to_string());
            args.push(format!("target={target}"));
        }

        // Build arguments
        let mut sorted_args: Vec<_> = self.config.build_args.iter().collect();
        sorted_args.sort_by_key(|(k, _)| (*k).clone());
        for (key, value) in &sorted_args {
            args.push("--opt".to_string());
            args.push(format!("build-arg:{key}={value}"));
        }

        // Output
        let output_type = self
            .config
            .output_type
            .as_deref()
            .unwrap_or("image");
        if !self.config.tags.is_empty() {
            let tag_names = self.config.tags.join(",");
            args.push("--output".to_string());
            args.push(format!(
                "type={output_type},name={tag_names},push={}",
                self.config.push
            ));
        } else {
            args.push("--output".to_string());
            args.push(format!("type={output_type}"));
        }

        // Cache import
        for cache in &self.config.cache_from {
            args.push("--import-cache".to_string());
            args.push(cache.clone());
        }

        // Cache export
        for cache in &self.config.cache_to {
            args.push("--export-cache".to_string());
            args.push(cache.clone());
        }

        args
    }

    /// Build environment variables for registry authentication.
    pub fn build_registry_env(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();
        for (host, auth) in &self.config.registry_auth {
            let sanitized_host = host.replace(['.', '-'], "_").to_uppercase();
            env.insert(
                format!("BUILDKIT_HOST_{sanitized_host}_USERNAME"),
                auth.username.clone(),
            );
            env.insert(
                format!("BUILDKIT_HOST_{sanitized_host}_PASSWORD"),
                auth.password.clone(),
            );
        }
        env
    }
}

/// Parse the image digest from buildctl output.
///
/// Looks for patterns like `exporting manifest sha256:<hex>` or
/// `digest: sha256:<hex>` in the combined output.
pub fn parse_digest(output: &str) -> Option<String> {
    let re = Regex::new(r"(?:exporting manifest |digest: )sha256:([a-f0-9]{64})").unwrap();
    re.captures(output)
        .map(|caps| format!("sha256:{}", &caps[1]))
}

#[async_trait]
impl StepBody for BuildkitStep {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let cmd_args = self.build_command();
        let registry_env = self.build_registry_env();

        let program = &cmd_args[0];
        let args = &cmd_args[1..];

        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args);

        // Set registry auth env vars.
        for (key, value) in &registry_env {
            cmd.env(key, value);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Execute with optional timeout.
        let output = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, cmd.output()).await {
                Ok(result) => result.map_err(|e| {
                    WfeError::StepExecution(format!("Failed to spawn buildctl: {e}"))
                })?,
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "buildctl timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            cmd.output()
                .await
                .map_err(|e| WfeError::StepExecution(format!("Failed to spawn buildctl: {e}")))?
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            return Err(WfeError::StepExecution(format!(
                "buildctl exited with code {code}\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        let step_name = context.step.name.as_deref().unwrap_or("unknown");

        let combined_output = format!("{stdout}\n{stderr}");
        let digest = parse_digest(&combined_output);

        let mut outputs = serde_json::Map::new();

        if let Some(ref digest) = digest {
            outputs.insert(
                format!("{step_name}.digest"),
                serde_json::Value::String(digest.clone()),
            );
        }

        if !self.config.tags.is_empty() {
            outputs.insert(
                format!("{step_name}.tags"),
                serde_json::Value::Array(
                    self.config
                        .tags
                        .iter()
                        .map(|t| serde_json::Value::String(t.clone()))
                        .collect(),
                ),
            );
        }

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
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    use crate::config::{BuildkitConfig, RegistryAuth, TlsConfig};

    fn minimal_config() -> BuildkitConfig {
        BuildkitConfig {
            dockerfile: "Dockerfile".to_string(),
            context: ".".to_string(),
            target: None,
            tags: vec![],
            build_args: HashMap::new(),
            cache_from: vec![],
            cache_to: vec![],
            push: false,
            output_type: None,
            buildkit_addr: "unix:///run/buildkit/buildkitd.sock".to_string(),
            tls: TlsConfig::default(),
            registry_auth: HashMap::new(),
            timeout_ms: None,
        }
    }

    #[test]
    fn build_command_minimal() {
        let step = BuildkitStep::new(minimal_config());
        let cmd = step.build_command();

        assert_eq!(cmd[0], "buildctl");
        assert_eq!(cmd[1], "--addr");
        assert_eq!(cmd[2], "unix:///run/buildkit/buildkitd.sock");
        assert_eq!(cmd[3], "build");
        assert_eq!(cmd[4], "--frontend");
        assert_eq!(cmd[5], "dockerfile.v0");
        assert_eq!(cmd[6], "--local");
        assert_eq!(cmd[7], "context=.");
        assert_eq!(cmd[8], "--local");
        assert_eq!(cmd[9], "dockerfile=.");
        assert_eq!(cmd[10], "--output");
        assert_eq!(cmd[11], "type=image");
    }

    #[test]
    fn build_command_with_target() {
        let mut config = minimal_config();
        config.target = Some("runtime".to_string());

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        let target_idx = cmd.iter().position(|a| a == "target=runtime").unwrap();
        assert_eq!(cmd[target_idx - 1], "--opt");
    }

    #[test]
    fn build_command_with_tags_and_push() {
        let mut config = minimal_config();
        config.tags = vec!["myapp:latest".to_string(), "myapp:v1.0".to_string()];
        config.push = true;

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        let output_idx = cmd.iter().position(|a| a == "--output").unwrap();
        assert_eq!(
            cmd[output_idx + 1],
            "type=image,name=myapp:latest,myapp:v1.0,push=true"
        );
    }

    #[test]
    fn build_command_with_build_args() {
        let mut config = minimal_config();
        config
            .build_args
            .insert("RUST_VERSION".to_string(), "1.78".to_string());
        config
            .build_args
            .insert("BUILD_MODE".to_string(), "release".to_string());

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        // Build args are sorted by key.
        let first_arg_idx = cmd
            .iter()
            .position(|a| a == "build-arg:BUILD_MODE=release")
            .unwrap();
        assert_eq!(cmd[first_arg_idx - 1], "--opt");

        let second_arg_idx = cmd
            .iter()
            .position(|a| a == "build-arg:RUST_VERSION=1.78")
            .unwrap();
        assert_eq!(cmd[second_arg_idx - 1], "--opt");
        assert!(first_arg_idx < second_arg_idx);
    }

    #[test]
    fn build_command_with_cache() {
        let mut config = minimal_config();
        config.cache_from = vec!["type=registry,ref=myapp:cache".to_string()];
        config.cache_to = vec!["type=registry,ref=myapp:cache,mode=max".to_string()];

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        let import_idx = cmd.iter().position(|a| a == "--import-cache").unwrap();
        assert_eq!(cmd[import_idx + 1], "type=registry,ref=myapp:cache");

        let export_idx = cmd.iter().position(|a| a == "--export-cache").unwrap();
        assert_eq!(
            cmd[export_idx + 1],
            "type=registry,ref=myapp:cache,mode=max"
        );
    }

    #[test]
    fn build_command_with_tls() {
        let mut config = minimal_config();
        config.tls = TlsConfig {
            ca: Some("/certs/ca.pem".to_string()),
            cert: Some("/certs/cert.pem".to_string()),
            key: Some("/certs/key.pem".to_string()),
        };

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        let ca_idx = cmd.iter().position(|a| a == "--tlscacert").unwrap();
        assert_eq!(cmd[ca_idx + 1], "/certs/ca.pem");

        let cert_idx = cmd.iter().position(|a| a == "--tlscert").unwrap();
        assert_eq!(cmd[cert_idx + 1], "/certs/cert.pem");

        let key_idx = cmd.iter().position(|a| a == "--tlskey").unwrap();
        assert_eq!(cmd[key_idx + 1], "/certs/key.pem");

        // TLS flags should come before "build" subcommand
        let build_idx = cmd.iter().position(|a| a == "build").unwrap();
        assert!(ca_idx < build_idx);
        assert!(cert_idx < build_idx);
        assert!(key_idx < build_idx);
    }

    #[test]
    fn build_command_with_registry_auth() {
        let mut config = minimal_config();
        config.registry_auth.insert(
            "ghcr.io".to_string(),
            RegistryAuth {
                username: "user".to_string(),
                password: "token".to_string(),
            },
        );

        let step = BuildkitStep::new(config);
        let env = step.build_registry_env();

        assert_eq!(
            env.get("BUILDKIT_HOST_GHCR_IO_USERNAME"),
            Some(&"user".to_string())
        );
        assert_eq!(
            env.get("BUILDKIT_HOST_GHCR_IO_PASSWORD"),
            Some(&"token".to_string())
        );
    }

    #[test]
    fn build_command_with_custom_dockerfile_path() {
        let mut config = minimal_config();
        config.dockerfile = "docker/Dockerfile.prod".to_string();

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        // Dockerfile directory should be "docker"
        let df_idx = cmd.iter().position(|a| a == "dockerfile=docker").unwrap();
        assert_eq!(cmd[df_idx - 1], "--local");

        // Non-default filename should be set
        let filename_idx = cmd
            .iter()
            .position(|a| a == "filename=Dockerfile.prod")
            .unwrap();
        assert_eq!(cmd[filename_idx - 1], "--opt");
    }

    #[test]
    fn build_command_with_output_type_local() {
        let mut config = minimal_config();
        config.output_type = Some("local".to_string());

        let step = BuildkitStep::new(config);
        let cmd = step.build_command();

        let output_idx = cmd.iter().position(|a| a == "--output").unwrap();
        assert_eq!(cmd[output_idx + 1], "type=local");
    }

    #[test]
    fn parse_digest_from_output() {
        let output = "some build output\nexporting manifest sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\ndone";
        let digest = parse_digest(output);
        assert_eq!(
            digest,
            Some(
                "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
                    .to_string()
            )
        );
    }

    #[test]
    fn parse_digest_with_digest_prefix() {
        let output = "digest: sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\n";
        let digest = parse_digest(output);
        assert_eq!(
            digest,
            Some(
                "sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                    .to_string()
            )
        );
    }

    #[test]
    fn parse_digest_missing_returns_none() {
        let output = "building image...\nall done!";
        let digest = parse_digest(output);
        assert_eq!(digest, None);
    }

    #[test]
    fn parse_digest_partial_hash_returns_none() {
        let output = "exporting manifest sha256:abcdef";
        let digest = parse_digest(output);
        assert_eq!(digest, None);
    }

    #[test]
    fn build_registry_env_sanitizes_host() {
        let mut config = minimal_config();
        config.registry_auth.insert(
            "my-registry.example.com".to_string(),
            RegistryAuth {
                username: "u".to_string(),
                password: "p".to_string(),
            },
        );

        let step = BuildkitStep::new(config);
        let env = step.build_registry_env();

        assert!(env.contains_key("BUILDKIT_HOST_MY_REGISTRY_EXAMPLE_COM_USERNAME"));
        assert!(env.contains_key("BUILDKIT_HOST_MY_REGISTRY_EXAMPLE_COM_PASSWORD"));
    }

    #[test]
    fn build_registry_env_empty_when_no_auth() {
        let step = BuildkitStep::new(minimal_config());
        let env = step.build_registry_env();
        assert!(env.is_empty());
    }
}
