//! Artifact model for OCI-compatible workflow inputs.

use serde::{Deserialize, Serialize};

/// A reference to an artifact stored in a content-addressed store.
///
/// The digest is a content hash (typically `sha256:<hex>`) that uniquely
/// identifies the artifact blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    /// Content digest, e.g. `sha256:abc123...`.
    pub digest: String,
}

impl ArtifactRef {
    /// Create a new artifact reference from a digest string.
    pub fn new(digest: impl Into<String>) -> Self {
        Self {
            digest: digest.into(),
        }
    }

    /// Extract the algorithm portion (e.g. `sha256`).
    pub fn algorithm(&self) -> Option<&str> {
        self.digest.split_once(':').map(|(algo, _)| algo)
    }

    /// Extract the hex-encoded hash portion.
    pub fn hash(&self) -> Option<&str> {
        self.digest.split_once(':').map(|(_, hash)| hash)
    }
}

/// An opaque blob stored in an artifact store.
///
/// This represents a single OCI image layer: a tar archive (usually
/// gzip-compressed) containing a filesystem tree.
#[derive(Debug, Clone)]
pub struct ArtifactBlob {
    /// Content digest (`sha256:...`).
    pub digest: String,
    /// Size in bytes.
    pub size: u64,
    /// OCI media type.
    pub media_type: String,
    /// Raw blob bytes.
    pub data: bytes::Bytes,
}

/// The well-known media type for a gzip-compressed OCI layer.
pub const MEDIA_TYPE_OCI_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

/// The well-known media type for an uncompressed OCI layer.
pub const MEDIA_TYPE_OCI_LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";

/// Reserved key used inside workflow data to mark an artifact reference.
pub const ARTIFACT_REF_KEY: &str = "__wfe_artifact";

/// Build an artifact-reference JSON value for insertion into workflow data.
pub fn artifact_ref_value(digest: &str) -> serde_json::Value {
    serde_json::json!({ ARTIFACT_REF_KEY: digest })
}

/// Check whether a JSON value is an artifact reference.
pub fn is_artifact_ref(value: &serde_json::Value) -> bool {
    value
        .as_object()
        .map(|obj| obj.contains_key(ARTIFACT_REF_KEY))
        .unwrap_or(false)
}

/// Extract the digest from an artifact-reference JSON value.
pub fn parse_artifact_ref(value: &serde_json::Value) -> Option<String> {
    value
        .as_object()
        .and_then(|obj| obj.get(ARTIFACT_REF_KEY))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_ref_roundtrip() {
        let r = ArtifactRef::new("sha256:abc123");
        assert_eq!(r.algorithm(), Some("sha256"));
        assert_eq!(r.hash(), Some("abc123"));
    }

    #[test]
    fn artifact_ref_value_is_artifact_ref() {
        let v = artifact_ref_value("sha256:def456");
        assert!(is_artifact_ref(&v));
        assert_eq!(parse_artifact_ref(&v), Some("sha256:def456".to_string()));
    }

    #[test]
    fn plain_string_is_not_artifact_ref() {
        assert!(!is_artifact_ref(&serde_json::Value::String("hello".into())));
    }
}
