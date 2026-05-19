//! Local filesystem-based artifact store using the OCI Image Layout.

use std::fmt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::models::{ArtifactRef, MEDIA_TYPE_OCI_LAYER_GZIP};
use crate::traits::ArtifactStore;
use crate::{Result, WfeError};

/// Metadata stored alongside each artifact blob.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ArtifactMeta {
    created_at: SystemTime,
    expires_at: Option<SystemTime>,
}

/// A local filesystem artifact store following the OCI Image Layout.
///
/// ```text
/// <root>/
///   oci-layout          → {"imageLayoutVersion": "1.0.0"}
///   index.json          → OCI Image Index
///   blobs/sha256/
///     <digest>          → opaque tar.gz bytes
///   meta/
///     <digest>.json     → { created_at, expires_at }
/// ```
#[derive(Debug, Clone)]
pub struct LocalArtifactStore {
    root: PathBuf,
    default_expiry: Option<Duration>,
}

impl LocalArtifactStore {
    /// Open (or create) a local artifact store at the given path.
    pub async fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let blobs = root.join("blobs/sha256");
        let meta = root.join("meta");
        fs::create_dir_all(&blobs).await.map_err(|e| io_err("create blob dir", e))?;
        fs::create_dir_all(&meta).await.map_err(|e| io_err("create meta dir", e))?;

        // Write oci-layout marker if missing.
        let oci_layout = root.join("oci-layout");
        if !oci_layout.exists() {
            fs::write(&oci_layout, r#"{"imageLayoutVersion": "1.0.0"}"#)
                .await
                .map_err(|e| io_err("write oci-layout", e))?;
        }

        Ok(Self {
            root,
            default_expiry: Some(Duration::from_secs(7 * 24 * 60 * 60)),
        })
    }

    /// Create with a custom default expiry.
    pub fn with_expiry(mut self, expiry: Option<Duration>) -> Self {
        self.default_expiry = expiry;
        self
    }

    fn blob_path(&self, digest: &str) -> PathBuf {
        let hash = digest.strip_prefix("sha256:").unwrap_or(digest);
        self.root.join("blobs/sha256").join(hash)
    }

    fn meta_path(&self, digest: &str) -> PathBuf {
        let hash = digest.strip_prefix("sha256:").unwrap_or(digest);
        self.root.join("meta").join(format!("{hash}.json"))
    }

    /// Remove expired artifacts lazily.
    async fn gc_expired(&self, digest: &str) -> Result<bool> {
        let meta_path = self.meta_path(digest);
        if !meta_path.exists() {
            return Ok(false);
        }
        let raw = fs::read(&meta_path)
            .await
            .map_err(|e| io_err("read meta", e))?;
        let meta: ArtifactMeta = serde_json::from_slice(&raw)?;
        if meta
            .expires_at
            .map(|t| SystemTime::now() > t)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(self.blob_path(digest)).await;
            let _ = fs::remove_file(&meta_path).await;
            return Ok(true);
        }
        Ok(false)
    }
}

#[async_trait]
impl ArtifactStore for LocalArtifactStore {
    async fn put(
        &self,
        mut reader: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
    ) -> Result<ArtifactRef> {
        let tmp_dir = self.root.join("tmp");
        fs::create_dir_all(&tmp_dir)
            .await
            .map_err(|e| io_err("create tmp dir", e))?;
        let tmp_path = tmp_dir.join(format!("{}.tmp", uuid::Uuid::new_v4()));

        let mut file = fs::File::create(&tmp_path)
            .await
            .map_err(|e| io_err("create tmp file", e))?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total: u64 = 0;

        loop {
            let n = reader
                .read(&mut buf)
                .await
                .map_err(|e| io_err("read input", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])
                .await
                .map_err(|e| io_err("write tmp file", e))?;
            total += n as u64;
        }
        file.flush()
            .await
            .map_err(|e| io_err("flush tmp file", e))?;
        drop(file);

        let hash = hex::encode(hasher.finalize());
        let digest = format!("sha256:{hash}");
        let dest = self.blob_path(&digest);

        // If already exists, deduplicate.
        if dest.exists() {
            fs::remove_file(&tmp_path)
                .await
                .map_err(|e| io_err("remove tmp file", e))?;
            return Ok(ArtifactRef::new(digest));
        }

        fs::rename(&tmp_path, &dest)
            .await
            .map_err(|e| io_err("rename blob", e))?;

        let meta = ArtifactMeta {
            created_at: SystemTime::now(),
            expires_at: self.default_expiry.map(|d| SystemTime::now() + d),
        };
        fs::write(self.meta_path(&digest), serde_json::to_vec(&meta)?)
            .await
            .map_err(|e| io_err("write meta", e))?;

        Ok(ArtifactRef::new(digest))
    }

    async fn get(
        &self,
        digest: &str,
    ) -> Result<Option<Pin<Box<dyn tokio::io::AsyncRead + Send>>>> {
        if self.gc_expired(digest).await? {
            return Ok(None);
        }
        let path = self.blob_path(digest);
        if !path.exists() {
            return Ok(None);
        }
        let file = fs::File::open(&path)
            .await
            .map_err(|e| io_err("open blob", e))?;
        Ok(Some(Box::pin(tokio::io::BufReader::new(file))))
    }

    async fn exists(&self, digest: &str) -> bool {
        if self.gc_expired(digest).await.unwrap_or(false) {
            return false;
        }
        self.blob_path(digest).exists()
    }

    fn default_expiry(&self) -> Option<Duration> {
        self.default_expiry
    }
}

/// Extract an artifact blob (gzip-compressed tar) into a directory.
///
/// The destination directory is created if it does not exist.
pub fn extract_artifact_to_dir<R: std::io::Read>(reader: R, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|e| io_err("create dest dir", e))?;
    let gz = GzDecoder::new(reader);
    let mut archive = Archive::new(gz);
    archive.set_preserve_permissions(true);
    archive.set_unpack_xattrs(true);
    archive
        .unpack(dest)
        .map_err(|e| WfeError::Other(format!("failed to unpack artifact: {e}").into()))?;
    Ok(())
}

fn io_err(ctx: &str, e: std::io::Error) -> WfeError {
    WfeError::Other(format!("artifact store io error ({ctx}): {e}").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn make_test_tar_gz() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let gz = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            let mut header = tar::Header::new_gnu();
            header.set_path("hello.txt").unwrap();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append(&header, Cursor::new(b"world")).unwrap();
            tar.finish().unwrap();
        }
        buf
    }

    #[tokio::test]
    async fn roundtrip_put_get() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalArtifactStore::open(tmp.path()).await.unwrap();
        let data = make_test_tar_gz();

        let aref = store
            .put(Box::pin(Cursor::new(data.clone())))
            .await
            .unwrap();
        assert!(aref.digest.starts_with("sha256:"));
        assert!(store.exists(&aref.digest).await);

        let mut reader = store.get(&aref.digest).await.unwrap().unwrap();
        let mut out = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut out)
            .await
            .unwrap();
        assert_eq!(out, data);
    }

    #[tokio::test]
    async fn dedup_same_content() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalArtifactStore::open(tmp.path()).await.unwrap();
        let data = make_test_tar_gz();

        let a1 = store.put(Box::pin(Cursor::new(data.clone()))).await.unwrap();
        let a2 = store.put(Box::pin(Cursor::new(data.clone()))).await.unwrap();
        assert_eq!(a1.digest, a2.digest);
    }

    #[tokio::test]
    async fn missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalArtifactStore::open(tmp.path()).await.unwrap();
        assert!(store
            .get("sha256:0000000000000000000000000000000000000000000000000000000000000000")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn expiry_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LocalArtifactStore::open(tmp.path())
            .await
            .unwrap()
            .with_expiry(Some(Duration::from_secs(0)));
        let data = make_test_tar_gz();

        let aref = store.put(Box::pin(Cursor::new(data))).await.unwrap();
        // Immediately expired.
        assert!(!store.exists(&aref.digest).await);
        assert!(store.get(&aref.digest).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn extract_artifact_to_dir_works() {
        let data = make_test_tar_gz();
        let dest = tempfile::tempdir().unwrap();
        extract_artifact_to_dir(Cursor::new(data), dest.path()).unwrap();
        let content = std::fs::read_to_string(dest.path().join("hello.txt")).unwrap();
        assert_eq!(content, "world");
    }
}
