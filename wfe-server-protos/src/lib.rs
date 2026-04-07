//! Generated gRPC stubs for the WFE workflow server API.
//!
//! Built from `proto/wfe/v1/wfe.proto`. Includes both server and client code.
//!
//! ```rust,ignore
//! use wfe_server_protos::wfe::v1::wfe_server::WfeServer;
//! use wfe_server_protos::wfe::v1::wfe_client::WfeClient;
//! ```

#![allow(clippy::all)]
#![allow(warnings)]

include!(concat!(env!("OUT_DIR"), "/mod.rs"));

pub use prost;
pub use prost_types;
pub use tonic;

/// Encoded file descriptor set for gRPC reflection.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wfe_descriptor.bin"));
