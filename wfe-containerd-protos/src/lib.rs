//! Generated gRPC stubs for the full containerd API.
//!
//! Built from the official containerd proto files at
//! <https://github.com/containerd/containerd/tree/main/api>.
//!
//! The module structure mirrors the protobuf package hierarchy:
//!
//! ```rust,ignore
//! use wfe_containerd_protos::containerd::services::containers::v1::containers_client::ContainersClient;
//! use wfe_containerd_protos::containerd::services::tasks::v1::tasks_client::TasksClient;
//! use wfe_containerd_protos::containerd::services::images::v1::images_client::ImagesClient;
//! use wfe_containerd_protos::containerd::services::version::v1::version_client::VersionClient;
//! use wfe_containerd_protos::containerd::types::Mount;
//! use wfe_containerd_protos::containerd::types::Descriptor;
//! ```

#![allow(clippy::all)]
#![allow(warnings)]

// tonic-build generates a mod.rs that defines the full module tree
// matching the protobuf package structure.
include!(concat!(env!("OUT_DIR"), "/mod.rs"));

/// Re-export tonic and prost for downstream convenience.
pub use prost;
pub use prost_types;
pub use tonic;
