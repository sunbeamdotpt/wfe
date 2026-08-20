//! Artifact volume abstraction for distributed workflow execution.
//!
//! An [`ArtifactVolume`] is a portable, in-memory collection of artifact bytes
//! resolved for a single workflow step. It can be materialized by any executor
//! type — shell (local dir), buildkit (build context), containerd
//! (content store + Diff.Apply), or kubernetes (ConfigMap/init-container).
//!
//! The serialized [`ArtifactVolumePackage`] form allows the volume to move
//! between processes (e.g., from a workflow scheduler to a remote worker).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bytes::Bytes;

use crate::models::parse_artifact_ref;
use crate::traits::ArtifactStore;
use crate::{Result, WfeError};

/// A portable collection of raw artifacts resolved for a single workflow step.
///
/// Each artifact is stored as the original `tar.gz` bytes fetched from the
/// artifact store. Executors materialize these bytes in their environment.
#[derive(Debug, Clone)]
pub struct ArtifactVolume {
    artifacts: HashMap<String, Bytes>,
}

impl ArtifactVolume {
    /// Create a volume from a pre-populated artifact map.
    pub fn from_artifacts(artifacts: HashMap<String, Bytes>) -> Self {
        Self { artifacts }
    }

    /// Resolve artifacts referenced by `inputs` from workflow data.
    ///
    /// For each `(name, _mount_point)` in `inputs`, looks up `name` in
    /// `workflow_data`. If the value is an artifact reference (`{"__wfe_artifact": "sha256:..."}`),
    /// fetches the bytes from the store and stores them under `name`.
    pub async fn resolve(
        inputs: &HashMap<String, String>,
        workflow_data: &serde_json::Value,
        store: &dyn ArtifactStore,
    ) -> Result<Self> {
        let data_obj = workflow_data
            .as_object()
            .ok_or_else(|| WfeError::StepExecution("workflow data is not an object".to_string()))?;

        let mut artifacts = HashMap::with_capacity(inputs.len());

        for name in inputs.keys() {
            let value = data_obj
                .get(name)
                .ok_or_else(|| WfeError::StepExecution(format!("input '{name}' not found in workflow data")))?;

            let digest = parse_artifact_ref(value)
                .ok_or_else(|| WfeError::StepExecution(format!("input '{name}' is not an artifact reference")))?;

            let reader = store
                .get(&digest)
                .await
                .map_err(|e| WfeError::StepExecution(format!("failed to get artifact '{name}': {e}")))?
                .ok_or_else(|| WfeError::StepExecution(format!("artifact not found: {digest}")))?;

            let mut bytes = Vec::new();
            let mut reader = reader;
            tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
                .await
                .map_err(|e| WfeError::StepExecution(format!("failed to read artifact '{name}': {e}")))?;

            artifacts.insert(name.clone(), Bytes::from(bytes));
        }

        Ok(Self { artifacts })
    }

    /// Same as `resolve`, but fetches **all** artifacts present in the
    /// workflow data, not just those named in `inputs`.
    pub async fn resolve_all(
        workflow_data: &serde_json::Value,
        store: &dyn ArtifactStore,
    ) -> Result<Self> {
        let data_obj = match workflow_data.as_object() {
            Some(obj) => obj,
            None => return Ok(Self::from_artifacts(HashMap::new())),
        };

        let mut artifacts = HashMap::new();

        for (name, value) in data_obj {
            if let Some(digest) = parse_artifact_ref(value) {
                let reader = store
                    .get(&digest)
                    .await
                    .map_err(|e| WfeError::StepExecution(format!("failed to get artifact '{name}': {e}")))?;

                if let Some(reader) = reader {
                    let mut bytes = Vec::new();
                    let mut reader = reader;
                    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes)
                        .await
                        .map_err(|e| {
                            WfeError::StepExecution(format!("failed to read artifact '{name}': {e}"))
                        })?;
                    artifacts.insert(name.clone(), Bytes::from(bytes));
                }
            }
        }

        Ok(Self { artifacts })
    }

    /// Get artifact bytes by input name.
    pub fn get(&self, name: &str) -> Option<&Bytes> {
        self.artifacts.get(name)
    }

    /// Iterate over `(name, bytes)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Bytes)> {
        self.artifacts.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Number of artifacts in this volume.
    pub fn len(&self) -> usize {
        self.artifacts.len()
    }

    /// True if the volume contains no artifacts.
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    /// Extract a single artifact to a directory.
    pub fn extract_one(&self, name: &str, dest: &Path) -> Result<()> {
        let bytes = self
            .artifacts
            .get(name)
            .ok_or_else(|| WfeError::StepExecution(format!("artifact '{name}' not in volume")))?;
        crate::local_artifact_store::extract_artifact_to_dir(std::io::Cursor::new(bytes), dest)
            .map_err(|e| WfeError::StepExecution(format!("failed to extract artifact '{name}': {e}")))
    }

    /// Extract all artifacts to subdirectories under `dest`, returning
    /// `name → extracted path`.
    pub fn extract_all(&self, dest: &Path) -> Result<HashMap<String, PathBuf>> {
        let mut result = HashMap::with_capacity(self.artifacts.len());
        for name in self.artifacts.keys() {
            let artifact_dest = dest.join(name);
            std::fs::create_dir_all(&artifact_dest).map_err(|e| {
                WfeError::StepExecution(format!(
                    "failed to create extraction dir for '{name}': {e}"
                ))
            })?;
            self.extract_one(name, &artifact_dest)?;
            result.insert(name.clone(), artifact_dest);
        }
        Ok(result)
    }

    /// Repackage an artifact with all paths rewritten under `prefix`.
    ///
    /// Used by containerd to create a rootfs-relative tar before calling
    /// `Diff.Apply`. If the artifact contains `file.txt` and prefix is
    /// `workspace/repo`, the result contains `workspace/repo/file.txt`.
    pub fn repackage_with_prefix(&self, name: &str, prefix: &str) -> Result<Bytes> {
        let bytes = self
            .artifacts
            .get(name)
            .ok_or_else(|| WfeError::StepExecution(format!("artifact '{name}' not in volume")))?;

        let mut output = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut output, flate2::Compression::default());
            let mut tar_out = tar::Builder::new(gz);

            let gz_in = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
            let mut tar_in = tar::Archive::new(gz_in);

            for entry in tar_in
                .entries()
                .map_err(|e| WfeError::StepExecution(format!("failed to read tar entries: {e}")))?
            {
                let mut entry = entry
                    .map_err(|e| WfeError::StepExecution(format!("failed to read tar entry: {e}")))?;
                let path = entry
                    .path()
                    .map_err(|e| WfeError::StepExecution(format!("invalid tar path: {e}")))?;

                let new_path = Path::new(prefix).join(&path);
                let new_path_str = new_path
                    .to_str()
                    .ok_or_else(|| WfeError::StepExecution("invalid utf-8 path".to_string()))?;

                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut data)
                    .map_err(|e| WfeError::StepExecution(format!("failed to read entry data: {e}")))?;

                let mut header = entry.header().clone();
                header.set_path(new_path_str).map_err(|e| {
                    WfeError::StepExecution(format!("failed to set tar path: {e}"))
                })?;

                tar_out
                    .append(&header, std::io::Cursor::new(data))
                    .map_err(|e| WfeError::StepExecution(format!("failed to append tar entry: {e}")))?;
            }

            tar_out
                .finish()
                .map_err(|e| WfeError::StepExecution(format!("failed to finish tar: {e}")))?;
        }

        Ok(Bytes::from(output))
    }

    /// Serialize to a transmissible package.
    pub fn package(&self) -> Result<ArtifactVolumePackage> {
        let mut output = Vec::new();
        {
            let mut tar_out = tar::Builder::new(&mut output);
            for (name, bytes) in &self.artifacts {
                let entry_name = format!("{name}.tar.gz");
                let mut header = tar::Header::new_gnu();
                header.set_path(&entry_name).map_err(|e| {
                    WfeError::StepExecution(format!("failed to set package path: {e}"))
                })?;
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                tar_out
                    .append(&header, std::io::Cursor::new(bytes))
                    .map_err(|e| WfeError::StepExecution(format!("failed to append package entry: {e}")))?;
            }
            tar_out
                .finish()
                .map_err(|e| WfeError::StepExecution(format!("failed to finish package: {e}")))?;
        }
        Ok(ArtifactVolumePackage(Bytes::from(output)))
    }
}

/// Serialized, transmissible form of an [`ArtifactVolume`].
///
/// A tar archive where each entry is `{name}.tar.gz` containing the
/// artifact's raw bytes.
#[derive(Debug, Clone)]
pub struct ArtifactVolumePackage(Bytes);

impl ArtifactVolumePackage {
    /// Deserialize back to an in-memory volume.
    pub fn unpack(&self) -> Result<ArtifactVolume> {
        let mut artifacts = HashMap::new();
        let mut tar_in = tar::Archive::new(std::io::Cursor::new(&self.0));

        for entry in tar_in
            .entries()
            .map_err(|e| WfeError::StepExecution(format!("failed to read package entries: {e}")))?
        {
            let mut entry = entry
                .map_err(|e| WfeError::StepExecution(format!("failed to read package entry: {e}")))?;
            let path = entry
                .path()
                .map_err(|e| WfeError::StepExecution(format!("invalid package path: {e}")))?;
            let name = path
                .to_str()
                .and_then(|s| s.strip_suffix(".tar.gz"))
                .ok_or_else(|| WfeError::StepExecution("invalid package entry name".to_string()))?
                .to_string();

            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)
                .map_err(|e| WfeError::StepExecution(format!("failed to read package entry: {e}")))?;
            artifacts.insert(name, Bytes::from(bytes));
        }

        Ok(ArtifactVolume { artifacts })
    }

    /// Raw tar bytes for streaming to remote executors.
    pub fn bytes(&self) -> &Bytes {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_volume_is_empty() {
        let vol = ArtifactVolume::from_artifacts(HashMap::new());
        assert!(vol.is_empty());
        assert_eq!(vol.len(), 0);
        assert!(vol.get("foo").is_none());
    }

    #[test]
    fn package_roundtrip() {
        let mut artifacts = HashMap::new();
        artifacts.insert("repo".to_string(), Bytes::from_static(b"fake-tar-gz-bytes"));
        let vol = ArtifactVolume::from_artifacts(artifacts);

        let package = vol.package().unwrap();
        let unpacked = package.unpack().unwrap();

        assert_eq!(unpacked.len(), 1);
        assert_eq!(unpacked.get("repo").unwrap().as_ref(), b"fake-tar-gz-bytes");
    }
}
