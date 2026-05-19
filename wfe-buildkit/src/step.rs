use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use regex::Regex;
use tokio_stream::StreamExt;
use tonic::transport::{Channel, Endpoint, Uri};
use wfe_buildkit_protos::moby::buildkit::v1::control_client::ControlClient;
use wfe_buildkit_protos::moby::buildkit::v1::{
    CacheOptions, CacheOptionsEntry, Exporter, SolveRequest, StatusRequest,
};
use wfe_core::WfeError;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

use crate::config::BuildkitConfig;

/// Result of a BuildKit solve operation.
#[derive(Debug, Clone)]
pub(crate) struct BuildResult {
    /// Image digest produced by the build, if any.
    pub digest: Option<String>,
    /// Full exporter response metadata from the daemon.
    #[allow(dead_code)]
    pub metadata: HashMap<String, String>,
}

/// A workflow step that builds container images via the BuildKit gRPC API.
pub struct BuildkitStep {
    config: BuildkitConfig,
}

impl BuildkitStep {
    /// Create a new BuildKit step from configuration.
    pub fn new(config: BuildkitConfig) -> Self {
        Self { config }
    }

    /// Connect to the BuildKit daemon and return a raw `ControlClient`.
    ///
    /// Supports Unix socket (`unix://`), TCP (`tcp://`), and HTTP (`http://`)
    /// endpoints.
    async fn connect(&self) -> Result<ControlClient<Channel>, WfeError> {
        let addr = &self.config.buildkit_addr;
        tracing::info!(addr = %addr, "connecting to BuildKit daemon");

        let channel = if addr.starts_with("unix://") {
            let socket_path = addr.strip_prefix("unix://").unwrap().to_string();

            // Verify the socket exists before attempting connection.
            if !Path::new(&socket_path).exists() {
                return Err(WfeError::StepExecution(format!(
                    "BuildKit socket not found: {socket_path}"
                )));
            }

            // tonic requires a dummy URI for Unix sockets; the actual path
            // is provided via the connector.
            Endpoint::try_from("http://[::]:50051")
                .map_err(|e| WfeError::StepExecution(format!("failed to create endpoint: {e}")))?
                .connect_with_connector(tower::service_fn(move |_: Uri| {
                    let path = socket_path.clone();
                    async move {
                        tokio::net::UnixStream::connect(path)
                            .await
                            .map(hyper_util::rt::TokioIo::new)
                    }
                }))
                .await
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "failed to connect to buildkitd via Unix socket at {addr}: {e}"
                    ))
                })?
        } else {
            // TCP or HTTP endpoint.
            let connect_addr = if addr.starts_with("tcp://") {
                addr.replacen("tcp://", "http://", 1)
            } else {
                addr.clone()
            };

            Endpoint::from_shared(connect_addr.clone())
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "invalid BuildKit endpoint {connect_addr}: {e}"
                    ))
                })?
                .timeout(std::time::Duration::from_secs(30))
                .connect()
                .await
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "failed to connect to buildkitd at {connect_addr}: {e}"
                    ))
                })?
        };

        Ok(ControlClient::new(channel))
    }

    /// Build frontend attributes from the current configuration.
    ///
    /// These attributes tell the BuildKit dockerfile frontend how to process
    /// the build: which file to use, which target stage, build arguments, etc.
    fn build_frontend_attrs(&self) -> HashMap<String, String> {
        let mut attrs = HashMap::new();

        // Dockerfile filename (relative to context).
        if self.config.dockerfile != "Dockerfile" {
            attrs.insert("filename".to_string(), self.config.dockerfile.clone());
        }

        // Target stage for multi-stage builds.
        if let Some(ref target) = self.config.target {
            attrs.insert("target".to_string(), target.clone());
        }

        // Build arguments (sorted for determinism).
        let mut sorted_args: Vec<_> = self.config.build_args.iter().collect();
        sorted_args.sort_by_key(|(k, _)| (*k).clone());
        for (key, value) in &sorted_args {
            attrs.insert(format!("build-arg:{key}"), value.to_string());
        }

        attrs
    }

    /// Build exporter configuration based on output type.
    fn build_exporters(&self) -> Vec<Exporter> {
        let output_type = self.config.output_type.as_deref().unwrap_or("image");

        match output_type {
            "docker" | "oci" | "tar" => {
                let mut export_attrs = HashMap::new();
                if !self.config.tags.is_empty() {
                    export_attrs.insert("name".to_string(), self.config.tags.join(","));
                }
                if let Some(ref dest) = self.config.output_dest {
                    export_attrs.insert("dest".to_string(), dest.clone());
                }
                let exporter_type = if output_type == "tar" { "docker" } else { output_type };
                vec![Exporter {
                    r#type: exporter_type.to_string(),
                    attrs: export_attrs,
                }]
            }
            "local" => {
                let mut export_attrs = HashMap::new();
                if let Some(ref dest) = self.config.output_dest {
                    export_attrs.insert("dest".to_string(), dest.clone());
                }
                vec![Exporter {
                    r#type: "local".to_string(),
                    attrs: export_attrs,
                }]
            }
            _ => {
                // Default "image" exporter.
                if self.config.tags.is_empty() {
                    return vec![];
                }
                let mut export_attrs = HashMap::new();
                export_attrs.insert("name".to_string(), self.config.tags.join(","));
                if self.config.push {
                    export_attrs.insert("push".to_string(), "true".to_string());
                }
                vec![Exporter {
                    r#type: "image".to_string(),
                    attrs: export_attrs,
                }]
            }
        }
    }

    /// Build cache options from the configuration.
    fn build_cache_options(&self) -> Option<CacheOptions> {
        if self.config.cache_from.is_empty() && self.config.cache_to.is_empty() {
            return None;
        }

        let imports = self
            .config
            .cache_from
            .iter()
            .map(|source| {
                let mut attrs = HashMap::new();
                attrs.insert("ref".to_string(), source.clone());
                CacheOptionsEntry {
                    r#type: "registry".to_string(),
                    attrs,
                }
            })
            .collect();

        let exports = self
            .config
            .cache_to
            .iter()
            .map(|dest| {
                let mut attrs = HashMap::new();
                attrs.insert("ref".to_string(), dest.clone());
                attrs.insert("mode".to_string(), "max".to_string());
                CacheOptionsEntry {
                    r#type: "registry".to_string(),
                    attrs,
                }
            })
            .collect();

        Some(CacheOptions {
            export_ref_deprecated: String::new(),
            import_refs_deprecated: vec![],
            export_attrs_deprecated: HashMap::new(),
            exports,
            imports,
        })
    }

    /// Execute the build against a connected BuildKit daemon.
    ///
    /// Constructs a `SolveRequest` using the dockerfile.v0 frontend and
    /// sends it via the Control gRPC service. The build context must be
    /// accessible to the daemon on its local filesystem (shared mount or
    /// same machine).
    ///
    /// # Session protocol
    ///
    /// TODO: For remote daemons where the build context is not on the same
    /// filesystem, a full session protocol implementation is needed to
    /// transfer files. Currently we rely on the context directory being
    /// available to buildkitd (e.g., via a shared mount in Lima/colima).
    async fn execute_build(
        &self,
        control: &mut ControlClient<Channel>,
    ) -> Result<BuildResult, WfeError> {
        let build_ref = format!("wfe-build-{}", uuid::Uuid::new_v4());
        let session_id = uuid::Uuid::new_v4().to_string();

        // Resolve the absolute context path.
        let abs_context = std::fs::canonicalize(&self.config.context).map_err(|e| {
            WfeError::StepExecution(format!(
                "failed to resolve context path {}: {e}",
                self.config.context
            ))
        })?;

        // Build frontend attributes with local context references.
        let mut frontend_attrs = self.build_frontend_attrs();

        // Point the frontend at the daemon-local context directory.
        // The "context" attr tells the dockerfile frontend where to find
        // the build context. For local builds we use the local source type
        // with a shared-key reference.
        let context_name = "context";
        let dockerfile_name = "dockerfile";

        frontend_attrs.insert("context".to_string(), format!("local://{context_name}"));
        frontend_attrs.insert(
            format!("local-sessionid:{context_name}"),
            session_id.clone(),
        );

        // Also provide the dockerfile source as a local reference.
        frontend_attrs.insert(
            "dockerfilekey".to_string(),
            format!("local://{dockerfile_name}"),
        );
        frontend_attrs.insert(
            format!("local-sessionid:{dockerfile_name}"),
            session_id.clone(),
        );

        let request = SolveRequest {
            r#ref: build_ref.clone(),
            definition: None,
            exporter_deprecated: String::new(),
            exporter_attrs_deprecated: HashMap::new(),
            session: session_id.clone(),
            frontend: "dockerfile.v0".to_string(),
            frontend_attrs,
            cache: self.build_cache_options(),
            entitlements: vec![],
            frontend_inputs: HashMap::new(),
            internal: false,
            source_policy: None,
            exporters: self.build_exporters(),
            enable_session_exporter: false,
            source_policy_session: String::new(),
        };

        // Attach session metadata headers so buildkitd knows which
        // session provides the local source content.
        let mut grpc_request = tonic::Request::new(request);
        let metadata = grpc_request.metadata_mut();

        // The x-docker-expose-session-uuid header tells buildkitd which
        // session owns the local sources. The x-docker-expose-session-grpc-method
        // header lists the gRPC methods the session implements.
        if let Ok(key) = "x-docker-expose-session-uuid"
            .parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>()
            && let Ok(val) =
                session_id.parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
        {
            metadata.insert(key, val);
        }

        // Advertise the filesync method so the daemon knows it can request
        // local file content from our session.
        if let Ok(key) = "x-docker-expose-session-grpc-method"
            .parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>()
        {
            if let Ok(val) = "/moby.filesync.v1.FileSync/DiffCopy"
                .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.append(key.clone(), val);
            }
            if let Ok(val) = "/moby.filesync.v1.Auth/Credentials"
                .parse::<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>()
            {
                metadata.append(key, val);
            }
        }

        tracing::info!(
            context = %abs_context.display(),
            session_id = %session_id,
            "sending solve request to BuildKit"
        );

        let response = control
            .solve(grpc_request)
            .await
            .map_err(|e| WfeError::StepExecution(format!("BuildKit solve failed: {e}")))?;
        let solve_response = response.into_inner();

        // Monitor progress (non-blocking, best effort).
        let status_request = StatusRequest {
            r#ref: build_ref.clone(),
        };
        if let Ok(stream_resp) = control.status(status_request).await {
            let mut stream = stream_resp.into_inner();
            while let Some(Ok(status)) = stream.next().await {
                for vertex in &status.vertexes {
                    if !vertex.name.is_empty() {
                        tracing::debug!(vertex = %vertex.name, "build progress");
                    }
                }
            }
        }

        // Extract digest.
        let digest = solve_response
            .exporter_response
            .get("containerimage.digest")
            .cloned();

        tracing::info!(digest = ?digest, "build completed");

        Ok(BuildResult {
            digest,
            metadata: solve_response.exporter_response,
        })
    }

    /// Build environment variables for registry authentication.
    ///
    /// This is still useful when the BuildKit daemon reads credentials from
    /// environment variables rather than session-based auth.
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

/// Parse the image digest from buildctl or BuildKit progress output.
///
/// Looks for patterns like `exporting manifest sha256:<hex>` or
/// `digest: sha256:<hex>` or the raw `containerimage.digest` value.
pub fn parse_digest(output: &str) -> Option<String> {
    let re = Regex::new(r"(?:exporting manifest |digest: )sha256:([a-f0-9]{64})").unwrap();
    re.captures(output)
        .map(|caps| format!("sha256:{}", &caps[1]))
}

/// Build the output data JSON object from step execution results.
///
/// Assembles a `serde_json::Value::Object` containing the step's stdout,
/// stderr, digest (if found), and tags (if any).
pub fn build_output_data(
    step_name: &str,
    stdout: &str,
    stderr: &str,
    digest: Option<&str>,
    tags: &[String],
    output_dest: Option<&str>,
) -> serde_json::Value {
    let mut outputs = serde_json::Map::new();

    if let Some(digest) = digest {
        outputs.insert(
            format!("{step_name}.digest"),
            serde_json::Value::String(digest.to_string()),
        );
    }

    if !tags.is_empty() {
        outputs.insert(
            format!("{step_name}.tags"),
            serde_json::Value::Array(
                tags.iter()
                    .map(|t| serde_json::Value::String(t.clone()))
                    .collect(),
            ),
        );
    }

    if let Some(dest) = output_dest {
        outputs.insert(
            format!("{step_name}.output_dest"),
            serde_json::Value::String(dest.to_string()),
        );
    }

    outputs.insert(
        format!("{step_name}.stdout"),
        serde_json::Value::String(stdout.to_string()),
    );
    outputs.insert(
        format!("{step_name}.stderr"),
        serde_json::Value::String(stderr.to_string()),
    );

    serde_json::Value::Object(outputs)
}

#[async_trait]
impl StepBody for BuildkitStep {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let step_name = context.step.name.as_deref().unwrap_or("unknown");

        // Connect to the BuildKit daemon.
        let mut control = self.connect().await?;

        tracing::info!(step = step_name, "submitting build to BuildKit");

        // Execute the build with optional timeout.
        let result = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, self.execute_build(&mut control)).await {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    return Err(WfeError::StepExecution(format!(
                        "BuildKit build timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            self.execute_build(&mut control).await?
        };

        // Extract digest from BuildResult.
        let digest = result.digest.clone();

        tracing::info!(
            step = step_name,
            digest = ?digest,
            "build completed"
        );

        let output_data = build_output_data(
            step_name,
            "", // gRPC builds don't produce traditional stdout
            "", // gRPC builds don't produce traditional stderr
            digest.as_deref(),
            &self.config.tags,
            self.config.output_dest.as_deref(),
        );

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
            output_dest: None,
            buildkit_addr: "unix:///run/buildkit/buildkitd.sock".to_string(),
            tls: TlsConfig::default(),
            registry_auth: HashMap::new(),
            timeout_ms: None,
        }
    }

    // ---------------------------------------------------------------
    // build_registry_env tests
    // ---------------------------------------------------------------

    #[test]
    fn build_registry_env_with_auth() {
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

    #[test]
    fn build_registry_env_multiple_registries() {
        let mut config = minimal_config();
        config.registry_auth.insert(
            "ghcr.io".to_string(),
            RegistryAuth {
                username: "gh_user".to_string(),
                password: "gh_pass".to_string(),
            },
        );
        config.registry_auth.insert(
            "docker.io".to_string(),
            RegistryAuth {
                username: "dh_user".to_string(),
                password: "dh_pass".to_string(),
            },
        );

        let step = BuildkitStep::new(config);
        let env = step.build_registry_env();

        assert_eq!(env.len(), 4);
        assert_eq!(env["BUILDKIT_HOST_GHCR_IO_USERNAME"], "gh_user");
        assert_eq!(env["BUILDKIT_HOST_GHCR_IO_PASSWORD"], "gh_pass");
        assert_eq!(env["BUILDKIT_HOST_DOCKER_IO_USERNAME"], "dh_user");
        assert_eq!(env["BUILDKIT_HOST_DOCKER_IO_PASSWORD"], "dh_pass");
    }

    // ---------------------------------------------------------------
    // parse_digest tests
    // ---------------------------------------------------------------

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
        let output =
            "digest: sha256:1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\n";
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
    fn parse_digest_empty_input() {
        assert_eq!(parse_digest(""), None);
    }

    #[test]
    fn parse_digest_wrong_prefix() {
        let output = "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        assert_eq!(parse_digest(output), None);
    }

    #[test]
    fn parse_digest_uppercase_hex_returns_none() {
        let output = "exporting manifest sha256:ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        assert_eq!(parse_digest(output), None);
    }

    #[test]
    fn parse_digest_multiline_with_noise() {
        let output = r#"
[+] Building 12.3s (8/8) FINISHED
 => exporting to image
 => exporting manifest sha256:aabbccdd0011223344556677aabbccdd0011223344556677aabbccdd00112233
 => done
"#;
        assert_eq!(
            parse_digest(output),
            Some(
                "sha256:aabbccdd0011223344556677aabbccdd0011223344556677aabbccdd00112233"
                    .to_string()
            )
        );
    }

    #[test]
    fn parse_digest_first_match_wins() {
        let hash1 = "a".repeat(64);
        let hash2 = "b".repeat(64);
        let output = format!("exporting manifest sha256:{hash1}\ndigest: sha256:{hash2}");
        let digest = parse_digest(&output).unwrap();
        assert_eq!(digest, format!("sha256:{hash1}"));
    }

    // ---------------------------------------------------------------
    // build_output_data tests
    // ---------------------------------------------------------------

    #[test]
    fn build_output_data_with_digest_and_tags() {
        let digest = "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let tags = vec!["myapp:latest".to_string(), "myapp:v1".to_string()];
        let result = build_output_data("build", "out", "err", Some(digest), &tags, None);

        let obj = result.as_object().unwrap();
        assert_eq!(obj["build.digest"], digest);
        assert_eq!(
            obj["build.tags"],
            serde_json::json!(["myapp:latest", "myapp:v1"])
        );
        assert_eq!(obj["build.stdout"], "out");
        assert_eq!(obj["build.stderr"], "err");
    }

    #[test]
    fn build_output_data_without_digest() {
        let result = build_output_data("step1", "hello", "", None, &[], None);

        let obj = result.as_object().unwrap();
        assert!(!obj.contains_key("step1.digest"));
        assert!(!obj.contains_key("step1.tags"));
        assert_eq!(obj["step1.stdout"], "hello");
        assert_eq!(obj["step1.stderr"], "");
    }

    #[test]
    fn build_output_data_with_digest_no_tags() {
        let digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let result = build_output_data("img", "ok", "warn", Some(digest), &[], None);

        let obj = result.as_object().unwrap();
        assert_eq!(obj["img.digest"], digest);
        assert!(!obj.contains_key("img.tags"));
        assert_eq!(obj["img.stdout"], "ok");
        assert_eq!(obj["img.stderr"], "warn");
    }

    #[test]
    fn build_output_data_no_digest_with_tags() {
        let tags = vec!["app:v2".to_string()];
        let result = build_output_data("s", "", "", None, &tags, None);

        let obj = result.as_object().unwrap();
        assert!(!obj.contains_key("s.digest"));
        assert_eq!(obj["s.tags"], serde_json::json!(["app:v2"]));
    }

    #[test]
    fn build_output_data_empty_strings() {
        let result = build_output_data("x", "", "", None, &[], None);
        let obj = result.as_object().unwrap();
        assert_eq!(obj["x.stdout"], "");
        assert_eq!(obj["x.stderr"], "");
        assert_eq!(obj.len(), 2);
    }

    #[test]
    fn build_output_data_with_output_dest() {
        let result = build_output_data("build", "out", "err", None, &[], Some("/tmp/image.tar"));
        let obj = result.as_object().unwrap();
        assert_eq!(obj["build.output_dest"], "/tmp/image.tar");
        assert_eq!(obj.len(), 3);
    }

    // ---------------------------------------------------------------
    // build_frontend_attrs tests
    // ---------------------------------------------------------------

    #[test]
    fn frontend_attrs_minimal() {
        let step = BuildkitStep::new(minimal_config());
        let attrs = step.build_frontend_attrs();
        // Default Dockerfile name is not included (only non-default).
        assert!(!attrs.contains_key("filename"));
        assert!(!attrs.contains_key("target"));
    }

    #[test]
    fn frontend_attrs_with_target() {
        let mut config = minimal_config();
        config.target = Some("runtime".to_string());
        let step = BuildkitStep::new(config);
        let attrs = step.build_frontend_attrs();
        assert_eq!(attrs.get("target"), Some(&"runtime".to_string()));
    }

    #[test]
    fn frontend_attrs_with_custom_dockerfile() {
        let mut config = minimal_config();
        config.dockerfile = "docker/Dockerfile.prod".to_string();
        let step = BuildkitStep::new(config);
        let attrs = step.build_frontend_attrs();
        assert_eq!(
            attrs.get("filename"),
            Some(&"docker/Dockerfile.prod".to_string())
        );
    }

    #[test]
    fn frontend_attrs_with_build_args() {
        let mut config = minimal_config();
        config
            .build_args
            .insert("RUST_VERSION".to_string(), "1.78".to_string());
        config
            .build_args
            .insert("BUILD_MODE".to_string(), "release".to_string());
        let step = BuildkitStep::new(config);
        let attrs = step.build_frontend_attrs();
        assert_eq!(
            attrs.get("build-arg:BUILD_MODE"),
            Some(&"release".to_string())
        );
        assert_eq!(
            attrs.get("build-arg:RUST_VERSION"),
            Some(&"1.78".to_string())
        );
    }

    // ---------------------------------------------------------------
    // build_exporters tests
    // ---------------------------------------------------------------

    #[test]
    fn exporters_empty_when_no_tags() {
        let step = BuildkitStep::new(minimal_config());
        assert!(step.build_exporters().is_empty());
    }

    #[test]
    fn exporters_with_tags_and_push() {
        let mut config = minimal_config();
        config.tags = vec!["myapp:latest".to_string(), "myapp:v1.0".to_string()];
        config.push = true;
        let step = BuildkitStep::new(config);
        let exporters = step.build_exporters();
        assert_eq!(exporters.len(), 1);
        assert_eq!(exporters[0].r#type, "image");
        assert_eq!(
            exporters[0].attrs.get("name"),
            Some(&"myapp:latest,myapp:v1.0".to_string())
        );
        assert_eq!(exporters[0].attrs.get("push"), Some(&"true".to_string()));
    }

    #[test]
    fn exporters_with_tags_no_push() {
        let mut config = minimal_config();
        config.tags = vec!["myapp:latest".to_string()];
        config.push = false;
        let step = BuildkitStep::new(config);
        let exporters = step.build_exporters();
        assert_eq!(exporters.len(), 1);
        assert!(!exporters[0].attrs.contains_key("push"));
    }

    #[test]
    fn exporters_docker_with_tags_and_dest() {
        let mut config = minimal_config();
        config.output_type = Some("docker".to_string());
        config.output_dest = Some("/tmp/out.tar".to_string());
        config.tags = vec!["myapp:latest".to_string()];
        let step = BuildkitStep::new(config);
        let exporters = step.build_exporters();
        assert_eq!(exporters.len(), 1);
        assert_eq!(exporters[0].r#type, "docker");
        assert_eq!(exporters[0].attrs.get("name"), Some(&"myapp:latest".to_string()));
        assert_eq!(exporters[0].attrs.get("dest"), Some(&"/tmp/out.tar".to_string()));
    }

    #[test]
    fn exporters_tar_maps_to_docker() {
        let mut config = minimal_config();
        config.output_type = Some("tar".to_string());
        config.output_dest = Some("/tmp/image.tar".to_string());
        config.tags = vec!["app:v1".to_string()];
        let step = BuildkitStep::new(config);
        let exporters = step.build_exporters();
        assert_eq!(exporters.len(), 1);
        assert_eq!(exporters[0].r#type, "docker");
        assert_eq!(exporters[0].attrs.get("dest"), Some(&"/tmp/image.tar".to_string()));
    }

    #[test]
    fn exporters_local_with_dest() {
        let mut config = minimal_config();
        config.output_type = Some("local".to_string());
        config.output_dest = Some("/tmp/rootfs".to_string());
        let step = BuildkitStep::new(config);
        let exporters = step.build_exporters();
        assert_eq!(exporters.len(), 1);
        assert_eq!(exporters[0].r#type, "local");
        assert_eq!(exporters[0].attrs.get("dest"), Some(&"/tmp/rootfs".to_string()));
        assert!(!exporters[0].attrs.contains_key("name"));
    }

    #[test]
    fn exporters_docker_without_dest() {
        let mut config = minimal_config();
        config.output_type = Some("docker".to_string());
        config.tags = vec!["myapp:latest".to_string()];
        let step = BuildkitStep::new(config);
        let exporters = step.build_exporters();
        assert_eq!(exporters.len(), 1);
        assert_eq!(exporters[0].r#type, "docker");
        assert!(!exporters[0].attrs.contains_key("dest"));
    }

    // ---------------------------------------------------------------
    // build_cache_options tests
    // ---------------------------------------------------------------

    #[test]
    fn cache_options_none_when_empty() {
        let step = BuildkitStep::new(minimal_config());
        assert!(step.build_cache_options().is_none());
    }

    #[test]
    fn cache_options_with_imports_and_exports() {
        let mut config = minimal_config();
        config.cache_from = vec!["type=registry,ref=myapp:cache".to_string()];
        config.cache_to = vec!["type=registry,ref=myapp:cache,mode=max".to_string()];
        let step = BuildkitStep::new(config);
        let opts = step.build_cache_options().unwrap();
        assert_eq!(opts.imports.len(), 1);
        assert_eq!(opts.exports.len(), 1);
        assert_eq!(opts.imports[0].r#type, "registry");
        assert_eq!(opts.exports[0].r#type, "registry");
    }

    // ---------------------------------------------------------------
    // connect helper tests
    // ---------------------------------------------------------------

    #[test]
    fn tcp_addr_converted_to_http() {
        let mut config = minimal_config();
        config.buildkit_addr = "tcp://buildkitd:1234".to_string();
        let step = BuildkitStep::new(config);
        assert_eq!(step.config.buildkit_addr, "tcp://buildkitd:1234");
    }

    #[test]
    fn unix_addr_preserved() {
        let config = minimal_config();
        let step = BuildkitStep::new(config);
        assert!(step.config.buildkit_addr.starts_with("unix://"));
    }

    #[tokio::test]
    async fn connect_to_missing_unix_socket_returns_error() {
        let mut config = minimal_config();
        config.buildkit_addr = "unix:///tmp/nonexistent-wfe-test.sock".to_string();
        let step = BuildkitStep::new(config);
        let err = step.connect().await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("socket not found"),
            "expected 'socket not found' error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn connect_to_invalid_tcp_returns_error() {
        let mut config = minimal_config();
        config.buildkit_addr = "tcp://127.0.0.1:1".to_string();
        let step = BuildkitStep::new(config);
        let err = step.connect().await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("failed to connect"),
            "expected connection error, got: {msg}"
        );
    }

    // ---------------------------------------------------------------
    // BuildkitStep construction tests
    // ---------------------------------------------------------------

    #[test]
    fn new_step_stores_config() {
        let config = minimal_config();
        let step = BuildkitStep::new(config.clone());
        assert_eq!(step.config.dockerfile, "Dockerfile");
        assert_eq!(step.config.context, ".");
    }
}
