//! wfe-kubernetes — Kubernetes step executor and service provider for WFE.
/// Resource cleanup utilities.
pub mod cleanup;
/// Kubernetes API client wrapper.
pub mod client;
/// Cluster and step configuration types.
pub mod config;
/// Pod log streaming.
pub mod logs;
/// YAML manifest generation.
pub mod manifests;
/// Namespace management.
pub mod namespace;
/// Step output handling.
pub mod output;
/// Persistent volume claim helpers.
pub mod pvc;
/// Service YAML manifest generation.
pub mod service_manifests;
/// [`ServiceProvider`](wfe_core::traits::service::ServiceProvider) implementation.
pub mod service_provider;
/// [`StepBody`](wfe_core::traits::step::StepBody) implementation for Kubernetes jobs.
pub mod step;

pub use config::{ClusterConfig, KubernetesStepConfig};
pub use service_provider::KubernetesServiceProvider;
pub use step::KubernetesStep;
