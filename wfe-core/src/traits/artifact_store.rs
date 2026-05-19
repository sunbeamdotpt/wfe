//! Artifact store trait for content-addressed OCI layer storage.

use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use async_trait::async_trait;

use crate::models::ArtifactRef;
use crate::Result;

/// A content-addressed store for OCI-compatible artifact blobs.
///
/// Implementations may be local (filesystem CAS), remote (S3, registry), or
/// in-memory (for testing).
#[async_trait]
pub trait ArtifactStore: Send + Sync + fmt::Debug {
    /// Stream a blob into the store, returning its content-addressed reference.
    ///
    /// The implementation should compute the digest while streaming and
    /// verify that the stored bytes match.
    async fn put(
        &self,
        reader: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
    ) -> Result<ArtifactRef>;

    /// Open a blob for reading.
    ///
    /// Returns `None` if the digest is not known to this store.
    async fn get(
        &self,
        digest: &str,
    ) -> Result<Option<Pin<Box<dyn tokio::io::AsyncRead + Send>>>>;

    /// Check existence without opening.
    async fn exists(&self, digest: &str) -> bool;

    /// Default TTL for artifacts stored by this implementation.
    ///
    /// `None` means artifacts never expire.
    fn default_expiry(&self) -> Option<Duration> {
        Some(Duration::from_secs(7 * 24 * 60 * 60)) // 7 days
    }
}
