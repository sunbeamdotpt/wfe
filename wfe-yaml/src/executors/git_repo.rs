/**! Git-repo step executor.

Clones a git repository using `gix`, stores the working tree as a gzipped tar
artifact, and writes the artifact ref to workflow data so downstream steps can
mount it.

## Artifact override

If `config.input` is set and `workflow.data[input]` contains an artifact ref
(`{"__wfe_artifact": "sha256:..."}`), the step **skips the clone entirely** and
extracts the existing artifact to `config.path`. This is the intended way to
cache git checkouts across workflow runs.

## Why normal clones don't cache

The artifact store is content-addressed (SHA-256). Two tar archives of the same
git checkout almost always have different digests because tar captures
non-deterministic metadata (mtimes, directory ordering, permissions). The store
treats every clone as a new artifact. Only the artifact-override path provides
effective deduplication.
*/

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncRead;
use tokio::task;
use tracing::{debug, info};

use wfe_core::models::ArtifactRef;
use wfe_core::models::ExecutionResult;
use wfe_core::traits::ArtifactStore;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

/// Configuration for the `git-repo` step type.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitRepoConfig {
    /// Repository URL to clone.
    pub url: String,
    /// Branch to check out (optional; defaults to remote HEAD).
    #[serde(default)]
    pub branch: Option<String>,
    /// Specific commit hash to check out after clone (optional).
    ///
    /// The working tree stays at the branch tip; HEAD is detached to this
    /// commit. For most CI use cases the commit is the branch tip, so this
    /// is a no-op.
    #[serde(default)]
    pub commit: Option<String>,
    /// Shallow-clone depth (optional; default is full clone).
    #[serde(default)]
    pub depth: Option<u32>,
    /// Local path where the repo should be placed.
    /// If relative, resolved against the workflow working directory.
    pub path: String,
    /// Name of the output key to write the artifact ref into.
    #[serde(default = "default_output")]
    pub output: String,
    /// Name of an input key that can override this step with a pre-built artifact.
    ///
    /// When `workflow.data[input]` contains `{"__wfe_artifact":"sha256:..."}`,
    /// the step skips the clone and extracts that artifact to `path`.
    /// This is the only mechanism that provides effective caching for git
    /// checkouts because the artifact store is content-addressed and tar
    /// archives of the same checkout are non-deterministic.
    #[serde(default)]
    pub input: Option<String>,
}

fn default_output() -> String {
    "repo".to_string()
}

/// The `git-repo` step body.
pub struct GitRepoStep {
    /// Step configuration.
    pub config: GitRepoConfig,
}

impl GitRepoStep {
    /// Create a new git-repo step.
    pub fn new(config: GitRepoConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl StepBody for GitRepoStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        let step_id = context.execution_pointer.step_id.clone();

        let store = context.artifact_store.ok_or_else(|| wfe_core::WfeError::StepExecution(
            "git-repo step requires an artifact store".to_string()
        ))?;

        // Check for artifact override from workflow data
        if let Some(ref input_key) = self.config.input {
            if let Some(artifact_ref) = extract_artifact_ref(&context.workflow.data, input_key) {
                info!("git-repo step '{step_id}' using artifact override from input '{input_key}'");
                let dest = resolve_path(&context.workflow.data, &self.config.path)?;
                extract_artifact_to_dir(store, &artifact_ref, &dest).await?;
                // Write artifact ref to output key
                let mut data = context.workflow.data.clone();
                insert_artifact_ref(&mut data, &self.config.output, artifact_ref);
                return Ok(ExecutionResult {
                    proceed: true,
                    output_data: Some(data),
                    ..Default::default()
                });
            }
        }

        // Normal clone path
        info!("git-repo step '{step_id}' cloning {}", self.config.url);

        let dest = resolve_path(&context.workflow.data, &self.config.path)?;

        let url = self.config.url.clone();
        let branch = self.config.branch.clone();
        let commit = self.config.commit.clone();
        let depth = self.config.depth;
        let dest_clone = dest.clone();

        let repo_path = task::spawn_blocking(move || {
            clone_and_checkout(url, branch, commit, depth, &dest_clone)
        })
        .await
        .map_err(|e| wfe_core::WfeError::StepExecution(format!("Clone task panicked: {e}")))??;

        // Tar + gzip the repo and store as artifact
        let artifact_ref = tar_and_store(store, &repo_path).await?;
        debug!("git-repo step '{step_id}' stored artifact {}", artifact_ref.digest);

        let mut data = context.workflow.data.clone();
        insert_artifact_ref(&mut data, &self.config.output, artifact_ref);

        Ok(ExecutionResult {
            proceed: true,
            output_data: Some(data),
            ..Default::default()
        })
    }
}

fn clone_and_checkout(
    url: String,
    branch: Option<String>,
    commit: Option<String>,
    depth: Option<u32>,
    dest: &Path,
) -> wfe_core::Result<PathBuf> {
    use gix::progress::Discard;
    use std::sync::atomic::AtomicBool;

    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new("."))).map_err(|e| {
        wfe_core::WfeError::StepExecution(format!("Failed to create parent directory: {e}"))
    })?;

    let mut prep = gix::prepare_clone(url, dest).map_err(|e| {
        wfe_core::WfeError::StepExecution(format!("Failed to prepare clone: {e}"))
    })?;

    if let Some(d) = depth {
        let nz = std::num::NonZeroU32::new(d).unwrap_or_else(|| std::num::NonZeroU32::new(1).unwrap());
        prep = prep.with_shallow(gix::remote::fetch::Shallow::DepthAtRemote(nz));
    }

    if let Some(ref b) = branch {
        prep = prep.with_ref_name(Some(b.as_str())).map_err(|e| {
            wfe_core::WfeError::StepExecution(format!("Invalid branch name: {e}"))
        })?;
    }

    let should_interrupt = AtomicBool::new(false);
    let (mut checkout, _fetch_outcome) = prep.fetch_then_checkout(Discard, &should_interrupt).map_err(|e| {
        wfe_core::WfeError::StepExecution(format!("Fetch failed: {e}"))
    })?;

    let (checked_repo, _checkout_outcome) = checkout.main_worktree(Discard, &should_interrupt).map_err(|e| {
        wfe_core::WfeError::StepExecution(format!("Checkout failed: {e}"))
    })?;

    // If a specific commit was requested, detach HEAD to it.
    // Note: the working tree remains at the branch tip; for workflow use
    // cases where the commit is the branch tip this is a no-op.
    if let Some(ref c) = commit {
        let oid = gix::hash::ObjectId::from_hex(c.as_bytes()).map_err(|e| {
            wfe_core::WfeError::StepExecution(format!("Invalid commit hash: {e}"))
        })?;
        checked_repo.edit_reference(gix::refs::transaction::RefEdit {
            change: gix::refs::transaction::Change::Update {
                log: gix::refs::transaction::LogChange {
                    mode: gix::refs::transaction::RefLog::AndReference,
                    force_create_reflog: false,
                    message: format!("wfe: checkout {c}").into(),
                },
                expected: gix::refs::transaction::PreviousValue::Any,
                new: gix::refs::Target::Object(oid),
            },
            name: "HEAD".try_into().expect("valid"),
            deref: true,
        }).map_err(|e| {
            wfe_core::WfeError::StepExecution(format!("Failed to detach HEAD: {e}"))
        })?;
    }

    Ok(dest.to_path_buf())
}

async fn tar_and_store(store: &dyn ArtifactStore, path: &Path) -> wfe_core::Result<ArtifactRef> {
    let path = path.to_path_buf();
    let buf = task::spawn_blocking(move || {
        let mut buf = Vec::new();
        let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        tar.append_dir_all(".", &path).map_err(|e| {
            wfe_core::WfeError::StepExecution(format!("Failed to tar repo: {e}"))
        })?;
        let enc = tar.into_inner().map_err(|e| {
            wfe_core::WfeError::StepExecution(format!("Failed to finish tar: {e}"))
        })?;
        enc.finish().map_err(|e| {
            wfe_core::WfeError::StepExecution(format!("Failed to finish gzip: {e}"))
        })?;
        Ok::<Vec<u8>, wfe_core::WfeError>(buf)
    })
    .await
    .map_err(|e| wfe_core::WfeError::StepExecution(format!("Tar task panicked: {e}")))??;

    let reader: Pin<Box<dyn AsyncRead + Send>> = Box::pin(AsyncCursor(std::io::Cursor::new(buf)));
    store.put(reader).await
}

fn extract_artifact_ref(data: &serde_json::Value, key: &str) -> Option<ArtifactRef> {
    data.get(key).and_then(|v| {
        v.get("__wfe_artifact").and_then(|d| {
            d.as_str().map(|s| ArtifactRef { digest: s.to_string() })
        })
    })
}

fn insert_artifact_ref(data: &mut serde_json::Value, key: &str, artifact: ArtifactRef) {
    if let Some(map) = data.as_object_mut() {
        map.insert(key.to_string(), serde_json::json!({ "__wfe_artifact": artifact.digest }));
    }
}

async fn extract_artifact_to_dir(
    store: &dyn ArtifactStore,
    artifact: &ArtifactRef,
    dest: &Path,
) -> wfe_core::Result<()> {
    let mut reader = store.get(&artifact.digest).await?.ok_or_else(|| {
        wfe_core::WfeError::StepExecution(format!("Artifact {} not found in store", artifact.digest))
    })?;

    let mut bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
        .await
        .map_err(|e| wfe_core::WfeError::StepExecution(format!("failed to read artifact: {e}")))?;

    task::spawn_blocking({
        let dest = dest.to_path_buf();
        move || wfe_core::local_artifact_store::extract_artifact_to_dir(std::io::Cursor::new(bytes), &dest)
    })
    .await
    .map_err(|e| wfe_core::WfeError::StepExecution(format!("extract task panicked: {e}")))?
}

/// Simple wrapper to turn a sync `std::io::Cursor<Vec<u8>>` into a `tokio::io::AsyncRead`.
struct AsyncCursor(std::io::Cursor<Vec<u8>>);

impl AsyncRead for AsyncCursor {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let n = std::io::Read::read(&mut self.0, buf.initialize_unfilled())?;
        buf.advance(n);
        Poll::Ready(Ok(()))
    }
}

fn resolve_path(data: &serde_json::Value, path: &str) -> wfe_core::Result<PathBuf> {
    let p = Path::new(path);
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        let base = data
            .get("__wfe_working_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(".");
        Ok(Path::new(base).join(p))
    }
}
