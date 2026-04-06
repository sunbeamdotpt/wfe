use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use tonic::transport::{Channel, Endpoint, Uri};
use wfe_core::WfeError;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

use wfe_containerd_protos::containerd::services::containers::v1::{
    containers_client::ContainersClient, Container, CreateContainerRequest,
    DeleteContainerRequest, container::Runtime,
};
use wfe_containerd_protos::containerd::services::content::v1::{
    content_client::ContentClient, ReadContentRequest,
};
use wfe_containerd_protos::containerd::services::images::v1::{
    images_client::ImagesClient, GetImageRequest,
};
use wfe_containerd_protos::containerd::services::snapshots::v1::{
    snapshots_client::SnapshotsClient, MountsRequest, PrepareSnapshotRequest,
};
use wfe_containerd_protos::containerd::services::tasks::v1::{
    tasks_client::TasksClient, CreateTaskRequest, DeleteTaskRequest, StartRequest,
    WaitRequest,
};
use wfe_containerd_protos::containerd::services::version::v1::version_client::VersionClient;

use crate::config::ContainerdConfig;

/// Default containerd namespace.
const DEFAULT_NAMESPACE: &str = "default";

/// Default snapshotter for rootless containerd.
const DEFAULT_SNAPSHOTTER: &str = "overlayfs";

pub struct ContainerdStep {
    config: ContainerdConfig,
}

impl ContainerdStep {
    pub fn new(config: ContainerdConfig) -> Self {
        Self { config }
    }

    /// Connect to the containerd daemon and return a raw tonic `Channel`.
    ///
    /// Supports Unix socket paths (bare `/path` or `unix:///path`) and
    /// TCP/HTTP endpoints.
    pub(crate) async fn connect(addr: &str) -> Result<Channel, WfeError> {
        let channel = if addr.starts_with('/') || addr.starts_with("unix://") {
            let socket_path = addr
                .strip_prefix("unix://")
                .unwrap_or(addr)
                .to_string();

            if !Path::new(&socket_path).exists() {
                return Err(WfeError::StepExecution(format!(
                    "containerd socket not found: {socket_path}"
                )));
            }

            Endpoint::try_from("http://[::]:50051")
                .map_err(|e| {
                    WfeError::StepExecution(format!("failed to create endpoint: {e}"))
                })?
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
                        "failed to connect to containerd via Unix socket at {addr}: {e}"
                    ))
                })?
        } else {
            let connect_addr = if addr.starts_with("tcp://") {
                addr.replacen("tcp://", "http://", 1)
            } else {
                addr.to_string()
            };

            Endpoint::from_shared(connect_addr.clone())
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "invalid containerd endpoint {connect_addr}: {e}"
                    ))
                })?
                .timeout(std::time::Duration::from_secs(30))
                .connect()
                .await
                .map_err(|e| {
                    WfeError::StepExecution(format!(
                        "failed to connect to containerd at {connect_addr}: {e}"
                    ))
                })?
        };

        Ok(channel)
    }

    /// Check whether an image exists in containerd's image store.
    ///
    /// Image pulling via raw containerd gRPC is complex (content store +
    /// snapshots + transfer). For now we only verify the image exists and
    /// return an error if it does not. Images must be pre-pulled via
    /// `ctr image pull` or `nerdctl pull`.
    ///
    /// TODO: implement full image pull via TransferService or content ingest.
    async fn ensure_image(
        channel: &Channel,
        image: &str,
        namespace: &str,
    ) -> Result<(), WfeError> {
        let mut client = ImagesClient::new(channel.clone());

        let mut req = tonic::Request::new(GetImageRequest {
            name: image.to_string(),
        });
        req.metadata_mut().insert(
            "containerd-namespace",
            namespace.parse().unwrap(),
        );

        match client.get(req).await {
            Ok(_) => Ok(()),
            Err(status) => Err(WfeError::StepExecution(format!(
                "image '{image}' not found in containerd (namespace={namespace}). \
                 Pre-pull it with: ctr -n {namespace} image pull {image}  \
                 (gRPC status: {status})"
            ))),
        }
    }

    /// Resolve the snapshot chain ID for an image.
    ///
    /// This reads the image manifest and config from the content store to
    /// compute the chain ID of the topmost layer. The chain ID is used as
    /// the parent snapshot when preparing a writable rootfs for a container.
    ///
    /// Chain ID computation follows the OCI image spec:
    ///   chain_id[0] = diff_id[0]
    ///   chain_id[n] = sha256(chain_id[n-1] + " " + diff_id[n])
    async fn resolve_image_chain_id(
        channel: &Channel,
        image: &str,
        namespace: &str,
    ) -> Result<String, WfeError> {
        use sha2::{Sha256, Digest};

        // 1. Get the image record to find the manifest digest.
        let mut images_client = ImagesClient::new(channel.clone());
        let req = Self::with_namespace(
            GetImageRequest { name: image.to_string() },
            namespace,
        );
        let image_resp = images_client.get(req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to get image '{image}': {e}"))
        })?;
        let img = image_resp.into_inner().image.ok_or_else(|| {
            WfeError::StepExecution(format!("image '{image}' has no record"))
        })?;
        let target = img.target.ok_or_else(|| {
            WfeError::StepExecution(format!("image '{image}' has no target descriptor"))
        })?;

        // The target might be an index (multi-platform) or a manifest.
        // Read the content and determine based on mediaType.
        let manifest_digest = target.digest.clone();
        let manifest_bytes = Self::read_content(channel, &manifest_digest, namespace).await?;
        let manifest_json: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| WfeError::StepExecution(format!("failed to parse manifest: {e}")))?;

        // 2. If it's an index, pick the matching platform manifest.
        let manifest_json = if manifest_json.get("manifests").is_some() {
            // OCI image index — find the platform-matching manifest.
            let arch = std::env::consts::ARCH;
            let oci_arch = match arch {
                "aarch64" => "arm64",
                "x86_64" => "amd64",
                other => other,
            };
            let manifests = manifest_json["manifests"].as_array().ok_or_else(|| {
                WfeError::StepExecution("image index has no manifests array".to_string())
            })?;
            let platform_manifest = manifests.iter().find(|m| {
                m.get("platform")
                    .and_then(|p| p.get("architecture"))
                    .and_then(|a| a.as_str())
                    == Some(oci_arch)
            }).ok_or_else(|| {
                WfeError::StepExecution(format!(
                    "no manifest for architecture '{oci_arch}' in image index"
                ))
            })?;
            let digest = platform_manifest["digest"].as_str().ok_or_else(|| {
                WfeError::StepExecution("platform manifest has no digest".to_string())
            })?;
            let bytes = Self::read_content(channel, digest, namespace).await?;
            serde_json::from_slice(&bytes)
                .map_err(|e| WfeError::StepExecution(format!("failed to parse platform manifest: {e}")))?
        } else {
            manifest_json
        };

        // 3. Get the config digest from the manifest.
        let config_digest = manifest_json["config"]["digest"]
            .as_str()
            .ok_or_else(|| {
                WfeError::StepExecution("manifest has no config.digest".to_string())
            })?;

        // 4. Read the image config.
        let config_bytes = Self::read_content(channel, config_digest, namespace).await?;
        let config_json: serde_json::Value = serde_json::from_slice(&config_bytes)
            .map_err(|e| WfeError::StepExecution(format!("failed to parse image config: {e}")))?;

        // 5. Extract diff_ids and compute chain ID.
        let diff_ids = config_json["rootfs"]["diff_ids"]
            .as_array()
            .ok_or_else(|| {
                WfeError::StepExecution("image config has no rootfs.diff_ids".to_string())
            })?;

        if diff_ids.is_empty() {
            return Err(WfeError::StepExecution(
                "image has no layers (empty diff_ids)".to_string(),
            ));
        }

        let mut chain_id = diff_ids[0]
            .as_str()
            .ok_or_else(|| WfeError::StepExecution("diff_id is not a string".to_string()))?
            .to_string();

        for diff_id in &diff_ids[1..] {
            let diff = diff_id.as_str().ok_or_else(|| {
                WfeError::StepExecution("diff_id is not a string".to_string())
            })?;
            let mut hasher = Sha256::new();
            hasher.update(format!("{chain_id} {diff}"));
            chain_id = format!("sha256:{:x}", hasher.finalize());
        }

        tracing::debug!(image = image, chain_id = %chain_id, "resolved image chain ID");
        Ok(chain_id)
    }

    /// Read content from the containerd content store by digest.
    async fn read_content(
        channel: &Channel,
        digest: &str,
        namespace: &str,
    ) -> Result<Vec<u8>, WfeError> {
        use tokio_stream::StreamExt;

        let mut client = ContentClient::new(channel.clone());
        let req = Self::with_namespace(
            ReadContentRequest {
                digest: digest.to_string(),
                offset: 0,
                size: 0, // read all
            },
            namespace,
        );

        let mut stream = client.read(req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to read content {digest}: {e}"))
        })?.into_inner();

        let mut data = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                WfeError::StepExecution(format!("error reading content {digest}: {e}"))
            })?;
            data.extend_from_slice(&chunk.data);
        }

        Ok(data)
    }

    /// Build a minimal OCI runtime spec as a `prost_types::Any`.
    ///
    /// The spec is serialized as JSON and wrapped in a protobuf Any with
    /// the containerd OCI spec type URL.
    pub(crate) fn build_oci_spec(
        &self,
        merged_env: &HashMap<String, String>,
    ) -> prost_types::Any {
        // Build the args array for the process.
        let args: Vec<String> = if let Some(ref run) = self.config.run {
            vec!["/bin/sh".to_string(), "-c".to_string(), run.clone()]
        } else if let Some(ref command) = self.config.command {
            command.clone()
        } else {
            vec![]
        };

        // Build env in KEY=VALUE form.
        let env: Vec<String> = merged_env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        // Build mounts.
        let mut mounts = vec![
            serde_json::json!({
                "destination": "/proc",
                "type": "proc",
                "source": "proc",
                "options": ["nosuid", "noexec", "nodev"]
            }),
            serde_json::json!({
                "destination": "/dev",
                "type": "tmpfs",
                "source": "tmpfs",
                "options": ["nosuid", "strictatime", "mode=755", "size=65536k"]
            }),
            serde_json::json!({
                "destination": "/sys",
                "type": "sysfs",
                "source": "sysfs",
                "options": ["nosuid", "noexec", "nodev", "ro"]
            }),
        ];

        for vol in &self.config.volumes {
            let mut opts = vec!["rbind".to_string()];
            if vol.readonly {
                opts.push("ro".to_string());
            }
            mounts.push(serde_json::json!({
                "destination": vol.target,
                "type": "bind",
                "source": vol.source,
                "options": opts,
            }));
        }

        // Parse user / group.
        let (uid, gid) = parse_user_spec(&self.config.user);

        let mut process = serde_json::json!({
            "terminal": false,
            "user": {
                "uid": uid,
                "gid": gid,
            },
            "args": args,
            "env": env,
            "cwd": self.config.working_dir.as_deref().unwrap_or("/"),
        });

        // Add capabilities. When running as root, grant the default Docker
        // capability set so tools like apt-get work. Non-root gets nothing.
        let caps = if uid == 0 {
            serde_json::json!([
                "CAP_AUDIT_WRITE", "CAP_CHOWN", "CAP_DAC_OVERRIDE",
                "CAP_FOWNER", "CAP_FSETID", "CAP_KILL", "CAP_MKNOD",
                "CAP_NET_BIND_SERVICE", "CAP_NET_RAW", "CAP_SETFCAP",
                "CAP_SETGID", "CAP_SETPCAP", "CAP_SETUID", "CAP_SYS_CHROOT",
            ])
        } else {
            serde_json::json!([])
        };
        process["capabilities"] = serde_json::json!({
            "bounding": caps,
            "effective": caps,
            "inheritable": caps,
            "permitted": caps,
            "ambient": caps,
        });

        let spec = serde_json::json!({
            "ociVersion": "1.0.2",
            "process": process,
            "root": {
                "path": "rootfs",
                "readonly": false,
            },
            "mounts": mounts,
            "linux": {
                "namespaces": [
                    { "type": "pid" },
                    { "type": "ipc" },
                    { "type": "uts" },
                    { "type": "mount" },
                ],
            },
        });

        let json_bytes = serde_json::to_vec(&spec).expect("OCI spec serialization cannot fail");

        prost_types::Any {
            type_url: "types.containerd.io/opencontainers/runtime-spec/1/Spec".to_string(),
            value: json_bytes,
        }
    }

    /// Inject a `containerd-namespace` header into a tonic request.
    pub(crate) fn with_namespace<T>(req: T, namespace: &str) -> tonic::Request<T> {
        let mut request = tonic::Request::new(req);
        request.metadata_mut().insert(
            "containerd-namespace",
            namespace.parse().unwrap(),
        );
        request
    }

    /// Start a long-running service container via the containerd gRPC API.
    ///
    /// Used by `ContainerdServiceProvider` to provision infrastructure services.
    /// The container runs on the host network so its ports are accessible on 127.0.0.1.
    /// Unlike step execution, this does NOT wait for the container to exit.
    pub async fn run_service(
        addr: &str,
        container_id: &str,
        image: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<(), WfeError> {
        let namespace = DEFAULT_NAMESPACE;
        let channel = Self::connect(addr).await?;

        // Verify image exists.
        Self::ensure_image(&channel, image, namespace).await?;

        // Build a config for host-network service container.
        let config = ContainerdConfig {
            image: image.to_string(),
            command: None,
            run: None,
            env: env.clone(),
            volumes: vec![],
            working_dir: None,
            user: "0:0".to_string(),
            network: "host".to_string(),
            memory: None,
            cpu: None,
            pull: "if-not-present".to_string(),
            containerd_addr: addr.to_string(),
            cli: "nerdctl".to_string(),
            tls: Default::default(),
            registry_auth: Default::default(),
            timeout_ms: None,
        };

        let step = Self::new(config);
        let oci_spec = step.build_oci_spec(env);

        // Create container.
        let mut containers_client = ContainersClient::new(channel.clone());
        let create_req = Self::with_namespace(
            CreateContainerRequest {
                container: Some(Container {
                    id: container_id.to_string(),
                    image: image.to_string(),
                    runtime: Some(Runtime {
                        name: "io.containerd.runc.v2".to_string(),
                        options: None,
                    }),
                    spec: Some(oci_spec),
                    snapshotter: DEFAULT_SNAPSHOTTER.to_string(),
                    snapshot_key: container_id.to_string(),
                    labels: HashMap::new(),
                    created_at: None,
                    updated_at: None,
                    extensions: HashMap::new(),
                    sandbox: String::new(),
                }),
            },
            namespace,
        );
        containers_client.create(create_req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to create service container: {e}"))
        })?;

        // Prepare snapshot.
        let mut snapshots_client = SnapshotsClient::new(channel.clone());
        let mounts = {
            let mounts_req = Self::with_namespace(
                MountsRequest {
                    snapshotter: DEFAULT_SNAPSHOTTER.to_string(),
                    key: container_id.to_string(),
                },
                namespace,
            );
            match snapshots_client.mounts(mounts_req).await {
                Ok(resp) => resp.into_inner().mounts,
                Err(_) => {
                    let parent =
                        Self::resolve_image_chain_id(&channel, image, namespace).await?;
                    let prepare_req = Self::with_namespace(
                        PrepareSnapshotRequest {
                            snapshotter: DEFAULT_SNAPSHOTTER.to_string(),
                            key: container_id.to_string(),
                            parent,
                            labels: HashMap::new(),
                        },
                        namespace,
                    );
                    snapshots_client
                        .prepare(prepare_req)
                        .await
                        .map_err(|e| {
                            WfeError::StepExecution(format!("failed to prepare snapshot: {e}"))
                        })?
                        .into_inner()
                        .mounts
                }
            }
        };

        // Create and start task (no stdout/stderr capture for services).
        let mut tasks_client = TasksClient::new(channel.clone());
        let create_task_req = Self::with_namespace(
            CreateTaskRequest {
                container_id: container_id.to_string(),
                rootfs: mounts,
                stdin: String::new(),
                stdout: String::new(),
                stderr: String::new(),
                terminal: false,
                checkpoint: None,
                options: None,
                runtime_path: String::new(),
            },
            namespace,
        );
        tasks_client.create(create_task_req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to create service task: {e}"))
        })?;

        let start_req = Self::with_namespace(
            StartRequest {
                container_id: container_id.to_string(),
                exec_id: String::new(),
            },
            namespace,
        );
        tasks_client.start(start_req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to start service task: {e}"))
        })?;

        tracing::info!(container_id = %container_id, image = %image, "service container started");
        Ok(())
    }

    /// Stop and clean up a service container via the containerd gRPC API.
    pub async fn cleanup_service(addr: &str, container_id: &str) -> Result<(), WfeError> {
        let channel = Self::connect(addr).await?;
        Self::cleanup(&channel, container_id, DEFAULT_NAMESPACE).await
    }

    /// Parse `##wfe[output key=value]` lines from stdout.
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
}

/// Parse a "uid:gid" string into (u32, u32). Falls back to (65534, 65534).
fn parse_user_spec(user: &str) -> (u32, u32) {
    let parts: Vec<&str> = user.split(':').collect();
    if parts.len() == 2 {
        let uid = parts[0].parse().unwrap_or(65534);
        let gid = parts[1].parse().unwrap_or(65534);
        (uid, gid)
    } else {
        (65534, 65534)
    }
}

#[async_trait]
impl StepBody for ContainerdStep {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let step_name = context.step.name.as_deref().unwrap_or("unknown");
        let namespace = DEFAULT_NAMESPACE;

        // 1. Connect to containerd.
        let addr = &self.config.containerd_addr;
        tracing::info!(addr = %addr, "connecting to containerd daemon");
        let channel = Self::connect(addr).await?;

        // Verify connectivity.
        {
            let mut version_client = VersionClient::new(channel.clone());
            let req = Self::with_namespace((), namespace);
            match version_client.version(req).await {
                Ok(resp) => {
                    let v = resp.into_inner();
                    tracing::info!(
                        version = %v.version,
                        revision = %v.revision,
                        "connected to containerd"
                    );
                }
                Err(e) => {
                    return Err(WfeError::StepExecution(format!(
                        "containerd version check failed: {e}"
                    )));
                }
            }
        }

        // 2. Ensure image exists (based on pull policy).
        let should_check = !matches!(self.config.pull.as_str(), "never");
        if should_check {
            Self::ensure_image(&channel, &self.config.image, namespace).await?;
        }

        // Generate a unique container ID.
        let container_id = format!("wfe-{}", uuid::Uuid::new_v4());

        // 3. Merge environment variables.
        let mut merged_env: HashMap<String, String> = HashMap::new();
        if let Some(data_obj) = context.workflow.data.as_object() {
            for (key, value) in data_obj {
                let env_key = key.to_uppercase();
                let env_val = match value {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                merged_env.insert(env_key, env_val);
            }
        }
        // Config env overrides workflow data.
        for (key, value) in &self.config.env {
            merged_env.insert(key.clone(), value.clone());
        }

        // 4. Build OCI spec.
        let oci_spec = self.build_oci_spec(&merged_env);

        // 5. Create container.
        tracing::info!(container_id = %container_id, image = %self.config.image, "creating container");
        let mut containers_client = ContainersClient::new(channel.clone());
        let create_req = Self::with_namespace(
            CreateContainerRequest {
                container: Some(Container {
                    id: container_id.clone(),
                    image: self.config.image.clone(),
                    runtime: Some(Runtime {
                        name: "io.containerd.runc.v2".to_string(),
                        options: None,
                    }),
                    spec: Some(oci_spec),
                    snapshotter: DEFAULT_SNAPSHOTTER.to_string(),
                    snapshot_key: container_id.clone(),
                    labels: HashMap::new(),
                    created_at: None,
                    updated_at: None,
                    extensions: HashMap::new(),
                    sandbox: String::new(),
                }),
            },
            namespace,
        );

        containers_client.create(create_req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to create container: {e}"))
        })?;

        // 6. Prepare snapshot with the image's layers as parent.
        let mut snapshots_client = SnapshotsClient::new(channel.clone());

        let mounts = {
            // First try: see if a snapshot was already prepared for this container.
            let mounts_req = Self::with_namespace(
                MountsRequest {
                    snapshotter: DEFAULT_SNAPSHOTTER.to_string(),
                    key: container_id.clone(),
                },
                namespace,
            );

            match snapshots_client.mounts(mounts_req).await {
                Ok(resp) => resp.into_inner().mounts,
                Err(_) => {
                    // Resolve the image's chain ID to use as snapshot parent.
                    let parent = if should_check {
                        Self::resolve_image_chain_id(&channel, &self.config.image, namespace).await?
                    } else {
                        String::new()
                    };

                    let prepare_req = Self::with_namespace(
                        PrepareSnapshotRequest {
                            snapshotter: DEFAULT_SNAPSHOTTER.to_string(),
                            key: container_id.clone(),
                            parent,
                            labels: HashMap::new(),
                        },
                        namespace,
                    );
                    snapshots_client
                        .prepare(prepare_req)
                        .await
                        .map_err(|e| {
                            WfeError::StepExecution(format!(
                                "failed to prepare snapshot: {e}"
                            ))
                        })?
                        .into_inner()
                        .mounts
                }
            }
        };

        // 7. Create FIFO paths for stdout/stderr capture.
        // Use WFE_IO_DIR if set (e.g., a shared mount with a remote containerd daemon),
        // otherwise fall back to the system temp directory.
        let io_base = std::env::var("WFE_IO_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let tmp_dir = io_base.join(format!("wfe-io-{container_id}"));
        std::fs::create_dir_all(&tmp_dir).map_err(|e| {
            WfeError::StepExecution(format!("failed to create IO temp dir: {e}"))
        })?;

        let stdout_path = tmp_dir.join("stdout");
        let stderr_path = tmp_dir.join("stderr");

        // Create empty files for the shim to write stdout/stderr to.
        // We use regular files instead of FIFOs because FIFOs don't work
        // across filesystem boundaries (e.g., virtiofs mounts with Lima VMs).
        for path in [&stdout_path, &stderr_path] {
            let _ = std::fs::remove_file(path);
            std::fs::File::create(path).map_err(|e| {
                WfeError::StepExecution(format!("failed to create IO file {}: {e}", path.display()))
            })?;
            // Ensure the remote shim can write to it.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).ok();
            }
        }

        let stdout_str = stdout_path.to_string_lossy().to_string();
        let stderr_str = stderr_path.to_string_lossy().to_string();

        // 8. Create task.
        let mut tasks_client = TasksClient::new(channel.clone());

        let create_task_req = Self::with_namespace(
            CreateTaskRequest {
                container_id: container_id.clone(),
                rootfs: mounts,
                stdin: String::new(),
                stdout: stdout_str.clone(),
                stderr: stderr_str.clone(),
                terminal: false,
                checkpoint: None,
                options: None,
                runtime_path: String::new(),
            },
            namespace,
        );

        tasks_client.create(create_task_req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to create task: {e}"))
        })?;

        // Start the task.
        let start_req = Self::with_namespace(
            StartRequest {
                container_id: container_id.clone(),
                exec_id: String::new(),
            },
            namespace,
        );

        tasks_client.start(start_req).await.map_err(|e| {
            WfeError::StepExecution(format!("failed to start task: {e}"))
        })?;

        tracing::info!(container_id = %container_id, "task started");

        // 9. Wait for task completion (with optional timeout).
        let wait_req = Self::with_namespace(
            WaitRequest {
                container_id: container_id.clone(),
                exec_id: String::new(),
            },
            namespace,
        );

        let wait_result = if let Some(timeout_ms) = self.config.timeout_ms {
            let duration = std::time::Duration::from_millis(timeout_ms);
            match tokio::time::timeout(duration, tasks_client.wait(wait_req)).await {
                Ok(result) => result,
                Err(_) => {
                    // Attempt cleanup before returning timeout error.
                    let _ = Self::cleanup(
                        &channel,
                        &container_id,
                        namespace,
                    )
                    .await;
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                    return Err(WfeError::StepExecution(format!(
                        "container execution timed out after {timeout_ms}ms"
                    )));
                }
            }
        } else {
            tasks_client.wait(wait_req).await
        };

        let exit_status = match wait_result {
            Ok(resp) => resp.into_inner().exit_status,
            Err(e) => {
                let _ = Self::cleanup(&channel, &container_id, namespace).await;
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return Err(WfeError::StepExecution(format!(
                    "failed waiting for task: {e}"
                )));
            }
        };

        // 10. Read captured output from files.
        let stdout_content = tokio::fs::read_to_string(&stdout_path)
            .await
            .unwrap_or_default();
        let stderr_content = tokio::fs::read_to_string(&stderr_path)
            .await
            .unwrap_or_default();

        // 11. Cleanup: delete task, then container.
        if let Err(e) = Self::cleanup(&channel, &container_id, namespace).await {
            tracing::warn!(container_id = %container_id, error = %e, "cleanup failed");
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);

        // 12. Check exit status.
        let exit_code = exit_status as i32;
        if exit_code != 0 {
            return Err(WfeError::StepExecution(format!(
                "container exited with code {exit_code}\nstdout: {stdout_content}\nstderr: {stderr_content}"
            )));
        }

        // 13. Parse outputs and build result.
        let parsed = Self::parse_outputs(&stdout_content);
        let output_data =
            Self::build_output_data(step_name, &stdout_content, &stderr_content, exit_code, &parsed);

        Ok(ExecutionResult {
            proceed: true,
            output_data: Some(output_data),
            ..Default::default()
        })
    }
}

impl ContainerdStep {
    /// Delete the task and container, best-effort.
    pub(crate) async fn cleanup(
        channel: &Channel,
        container_id: &str,
        namespace: &str,
    ) -> Result<(), WfeError> {
        let mut tasks_client = TasksClient::new(channel.clone());
        let mut containers_client = ContainersClient::new(channel.clone());

        // Delete task (ignore errors — it may already be gone).
        let del_task_req = Self::with_namespace(
            DeleteTaskRequest {
                container_id: container_id.to_string(),
            },
            namespace,
        );
        let _ = tasks_client.delete(del_task_req).await;

        // Delete container.
        let del_container_req = Self::with_namespace(
            DeleteContainerRequest {
                id: container_id.to_string(),
            },
            namespace,
        );
        containers_client
            .delete(del_container_req)
            .await
            .map_err(|e| {
                WfeError::StepExecution(format!("failed to delete container: {e}"))
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TlsConfig, VolumeMountConfig};
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

    // ── parse_user_spec ────────────────────────────────────────────────

    #[test]
    fn parse_user_spec_normal() {
        assert_eq!(parse_user_spec("1000:1000"), (1000, 1000));
    }

    #[test]
    fn parse_user_spec_root() {
        assert_eq!(parse_user_spec("0:0"), (0, 0));
    }

    #[test]
    fn parse_user_spec_default() {
        assert_eq!(parse_user_spec("65534:65534"), (65534, 65534));
    }

    #[test]
    fn parse_user_spec_invalid_falls_back() {
        assert_eq!(parse_user_spec("abc"), (65534, 65534));
    }

    // ── build_oci_spec ─────────────────────────────────────────────────

    #[test]
    fn build_oci_spec_minimal() {
        let step = ContainerdStep::new(minimal_config());
        let env = HashMap::new();
        let spec = step.build_oci_spec(&env);

        assert_eq!(
            spec.type_url,
            "types.containerd.io/opencontainers/runtime-spec/1/Spec"
        );
        assert!(!spec.value.is_empty());

        // Deserialize and verify.
        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        assert_eq!(parsed["ociVersion"], "1.0.2");
        assert_eq!(parsed["process"]["args"][0], "/bin/sh");
        assert_eq!(parsed["process"]["args"][1], "-c");
        assert_eq!(parsed["process"]["args"][2], "echo hello");
        assert_eq!(parsed["process"]["user"]["uid"], 65534);
        assert_eq!(parsed["process"]["user"]["gid"], 65534);
        assert_eq!(parsed["process"]["cwd"], "/");
    }

    #[test]
    fn build_oci_spec_with_command() {
        let mut config = minimal_config();
        config.run = None;
        config.command = Some(vec!["echo".to_string(), "hello".to_string(), "world".to_string()]);
        let step = ContainerdStep::new(config);
        let spec = step.build_oci_spec(&HashMap::new());

        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        assert_eq!(parsed["process"]["args"][0], "echo");
        assert_eq!(parsed["process"]["args"][1], "hello");
        assert_eq!(parsed["process"]["args"][2], "world");
    }

    #[test]
    fn build_oci_spec_with_env() {
        let step = ContainerdStep::new(minimal_config());
        let env = HashMap::from([
            ("FOO".to_string(), "bar".to_string()),
            ("BAZ".to_string(), "qux".to_string()),
        ]);
        let spec = step.build_oci_spec(&env);

        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        let env_arr: Vec<String> = parsed["process"]["env"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();

        assert!(env_arr.contains(&"FOO=bar".to_string()));
        assert!(env_arr.contains(&"BAZ=qux".to_string()));
    }

    #[test]
    fn build_oci_spec_with_working_dir() {
        let mut config = minimal_config();
        config.working_dir = Some("/app".to_string());
        let step = ContainerdStep::new(config);
        let spec = step.build_oci_spec(&HashMap::new());

        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        assert_eq!(parsed["process"]["cwd"], "/app");
    }

    #[test]
    fn build_oci_spec_with_user() {
        let mut config = minimal_config();
        config.user = "1000:2000".to_string();
        let step = ContainerdStep::new(config);
        let spec = step.build_oci_spec(&HashMap::new());

        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        assert_eq!(parsed["process"]["user"]["uid"], 1000);
        assert_eq!(parsed["process"]["user"]["gid"], 2000);
    }

    #[test]
    fn build_oci_spec_with_volumes() {
        let mut config = minimal_config();
        config.volumes = vec![
            VolumeMountConfig {
                source: "/host/data".to_string(),
                target: "/container/data".to_string(),
                readonly: false,
            },
            VolumeMountConfig {
                source: "/host/config".to_string(),
                target: "/etc/config".to_string(),
                readonly: true,
            },
        ];
        let step = ContainerdStep::new(config);
        let spec = step.build_oci_spec(&HashMap::new());

        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        let mounts = parsed["mounts"].as_array().unwrap();
        // 3 default + 2 user = 5
        assert_eq!(mounts.len(), 5);

        let bind_mounts: Vec<&serde_json::Value> = mounts
            .iter()
            .filter(|m| m["type"] == "bind")
            .collect();
        assert_eq!(bind_mounts.len(), 2);

        let ro_mount = bind_mounts
            .iter()
            .find(|m| m["destination"] == "/etc/config")
            .unwrap();
        let opts: Vec<String> = ro_mount["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(opts.contains(&"ro".to_string()));
    }

    #[test]
    fn build_oci_spec_no_command_no_run() {
        let mut config = minimal_config();
        config.run = None;
        config.command = None;
        let step = ContainerdStep::new(config);
        let spec = step.build_oci_spec(&HashMap::new());

        let parsed: serde_json::Value = serde_json::from_slice(&spec.value).unwrap();
        assert!(parsed["process"]["args"].as_array().unwrap().is_empty());
    }

    // ── connect ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn connect_to_missing_unix_socket_returns_error() {
        let err = ContainerdStep::connect("/tmp/nonexistent-wfe-containerd-test.sock")
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("socket not found"),
            "expected 'socket not found' error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn connect_to_missing_unix_socket_with_scheme_returns_error() {
        let err =
            ContainerdStep::connect("unix:///tmp/nonexistent-wfe-containerd-test.sock")
                .await
                .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("socket not found"),
            "expected 'socket not found' error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn connect_to_invalid_tcp_returns_error() {
        let err = ContainerdStep::connect("tcp://127.0.0.1:1")
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("failed to connect"),
            "expected connection error, got: {msg}"
        );
    }

    // ── ContainerdStep::new ────────────────────────────────────────────

    #[test]
    fn new_creates_step_with_config() {
        let config = minimal_config();
        let step = ContainerdStep::new(config);
        assert_eq!(step.config.image, "alpine:3.18");
        assert_eq!(step.config.containerd_addr, "/run/containerd/containerd.sock");
    }

}

/// Integration tests that require a live containerd daemon.
#[cfg(test)]
mod e2e_tests {
    use super::*;

    /// Returns the containerd socket address if available, or None.
    fn containerd_addr() -> Option<String> {
        let addr = std::env::var("WFE_CONTAINERD_ADDR").unwrap_or_else(|_| {
            format!(
                "unix://{}/.lima/wfe-test/sock/containerd.sock",
                std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
            )
        });

        let socket_path = addr
            .strip_prefix("unix://")
            .unwrap_or(addr.as_str());

        if Path::new(socket_path).exists() {
            Some(addr)
        } else {
            None
        }
    }

    #[tokio::test]
    async fn e2e_version_check() {
        let Some(addr) = containerd_addr() else {
            eprintln!("SKIP: containerd socket not available");
            return;
        };

        let channel = ContainerdStep::connect(&addr).await.unwrap();
        let mut client = VersionClient::new(channel);

        let req = ContainerdStep::with_namespace((), DEFAULT_NAMESPACE);
        let resp = client.version(req).await.unwrap();
        let version = resp.into_inner();

        assert!(!version.version.is_empty(), "version should not be empty");
        assert!(!version.revision.is_empty(), "revision should not be empty");
        eprintln!("containerd version={} revision={}", version.version, version.revision);
    }
}
