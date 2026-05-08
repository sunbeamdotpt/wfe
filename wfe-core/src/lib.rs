#![warn(missing_docs)]
//! wfe-core — Core traits, models, builder, executor, and primitives for the WFE
//! persistent workflow engine.
/// Builder.
pub mod builder;
/// Error.
pub mod error;
/// Executor.
pub mod executor;
/// Models.
pub mod models;
/// Primitives.
pub mod primitives;
/// Traits.
pub mod traits;

#[cfg(any(test, feature = "test-support"))]
/// Test support.
pub mod test_support;

pub use error::{Result, WfeError};
