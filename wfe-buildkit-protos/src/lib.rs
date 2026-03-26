//! Generated gRPC stubs for the full BuildKit API.
//!
//! Built from the official BuildKit proto files at
//! <https://github.com/moby/buildkit>.
//!
//! ```rust,ignore
//! use wfe_buildkit_protos::moby::buildkit::v1::control_client::ControlClient;
//! use wfe_buildkit_protos::moby::buildkit::v1::StatusResponse;
//! ```

#![allow(clippy::all)]
#![allow(warnings)]

include!(concat!(env!("OUT_DIR"), "/mod.rs"));

/// Re-export tonic and prost for downstream convenience.
pub use prost;
pub use prost_types;
pub use tonic;
