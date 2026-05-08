//! wfectl: command-line client for wfe-server.
//!
//! `wfectl` is the CLI for interacting with a running `wfe-server`. It supports
//! OAuth2 authentication, multiple output formats (table, JSON, YAML), and all
//! major workflow operations.
//!
//! # Usage
//! ```text
//! wfectl run ./my-workflow.yaml
//! wfectl get <workflow-id>
//! wfectl list
//! wfectl cancel <workflow-id>
//! wfectl logs <workflow-id>
//! ```
//!
//! # Modules
//! | Module | Purpose |
//! |--------|---------|
//! | [`auth`](auth) | OAuth2 token and credential management |
//! | [`client`](client) | gRPC client builder |
//! | [`commands`](commands) | CLI subcommands (run, get, list, cancel, etc.) |
//! | [`config`](config) | Configuration file and context management |
//! | [`output`](output) | Output formatting (table, JSON, YAML) |
//! | [`struct_util`](struct_util) | Struct-to-JSON conversion utilities |

/// OAuth2 token and credential management.
pub mod auth;
/// gRPC client builder.
pub mod client;
/// CLI subcommands.
pub mod commands;
/// Configuration file and context management.
pub mod config;
/// Output formatting (table, JSON, YAML).
pub mod output;
/// Struct-to-JSON conversion utilities.
pub mod struct_util;
