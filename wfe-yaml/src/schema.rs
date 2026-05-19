use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A condition in YAML that determines whether a step executes.
///
/// Uses `#[serde(untagged)]` so serde tries each variant in order.
/// A comparison has a `field:` key; a combinator has `all:/any:/none:/one_of:/not:`.
/// Comparison is listed first because it is more specific (requires `field`).
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
#[serde(untagged)]
pub enum YamlCondition {
    /// Leaf comparison (has a `field:` key).
    Comparison(Box<YamlComparison>),
    /// Combinator with sub-conditions.
    Combinator(YamlCombinator),
}

/// A combinator condition containing sub-conditions.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct YamlCombinator {
    #[serde(default)]
    /// All.
    pub all: Option<Vec<YamlCondition>>,
    #[serde(default)]
    /// Any.
    pub any: Option<Vec<YamlCondition>>,
    #[serde(default)]
    /// None.
    pub none: Option<Vec<YamlCondition>>,
    #[serde(default)]
    /// One of.
    pub one_of: Option<Vec<YamlCondition>>,
    #[serde(default)]
    /// Not.
    pub not: Option<Box<YamlCondition>>,
}

/// A leaf comparison condition that compares a field value.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct YamlComparison {
    /// Field.
    pub field: String,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Equals.
    pub equals: Option<serde_yaml::Value>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Not equals.
    pub not_equals: Option<serde_yaml::Value>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Gt.
    pub gt: Option<serde_yaml::Value>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Gte.
    pub gte: Option<serde_yaml::Value>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Lt.
    pub lt: Option<serde_yaml::Value>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Lte.
    pub lte: Option<serde_yaml::Value>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    /// Contains.
    pub contains: Option<serde_yaml::Value>,
    #[serde(default)]
    /// Is null.
    pub is_null: Option<bool>,
    #[serde(default)]
    /// Is not null.
    pub is_not_null: Option<bool>,
}

/// Top-level YAML file structure supporting both single and multi-workflow files.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlWorkflowFile {
    /// Single workflow (backward compatible).
    pub workflow: Option<WorkflowSpec>,
    /// Multiple workflows in one file.
    pub workflows: Option<Vec<WorkflowSpec>>,
}

/// Legacy single-workflow top-level structure. Kept for backward compatibility
/// with code that deserializes `YamlWorkflow` directly.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlWorkflow {
    /// Workflow.
    pub workflow: WorkflowSpec,
}

/// A complete workflow definition.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct WorkflowSpec {
    /// Unique workflow identifier (slug, e.g. "ci"). Primary lookup key.
    pub id: String,
    /// Optional human-friendly display name shown in UIs and listings
    /// (e.g. "Continuous Integration"). Defaults to the slug `id` when unset.
    #[serde(default)]
    pub name: Option<String>,
    /// Workflow version number.
    pub version: u32,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: Option<String>,
    /// Default error handling behavior for all steps.
    #[serde(default)]
    pub error_behavior: Option<YamlErrorBehavior>,
    /// The steps that make up this workflow.
    pub steps: Vec<YamlStep>,
    /// Typed input schema: { field_name: type_string }.
    /// Example: `"repo_url": "string"`, `"tags": "list<string>"`.
    #[serde(default)]
    pub inputs: HashMap<String, String>,
    /// Typed output schema: { field_name: type_string }.
    #[serde(default)]
    pub outputs: HashMap<String, String>,
    /// Infrastructure services required by this workflow (databases, caches, etc.).
    #[serde(default)]
    pub services: HashMap<String, YamlService>,
    /// Optional persistent volume shared across every step in this workflow
    /// run, including sub-workflows. Declared once on the top-level
    /// orchestrator (e.g. `ci`); ignored on non-root workflows. The Kubernetes
    /// executor provisions a single PVC per top-level run and mounts it on
    /// every step container at `mount_path` so steps like `git clone` in one
    /// sub-workflow are visible to `cargo fmt --check` in another.
    #[serde(default)]
    pub shared_volume: Option<YamlSharedVolume>,
    /// Allow unknown top-level keys (e.g. `_templates`) for YAML anchors.
    #[serde(flatten)]
    #[schemars(skip)]
    pub _extra: HashMap<String, serde_yaml::Value>,
}

/// Shared volume declaration, YAML form.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlSharedVolume {
    /// Absolute path to mount the volume at inside every step container.
    /// Defaults to `/workspace` when unset.
    #[serde(default = "default_shared_volume_mount")]
    pub mount_path: String,
    /// Optional size (e.g. `"20Gi"`). When unset the backend falls back
    /// to its configured default.
    #[serde(default)]
    pub size: Option<String>,
}

fn default_shared_volume_mount() -> String {
    "/workspace".to_string()
}

/// A service definition in YAML format.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlService {
    /// Container image to run (e.g., "postgres:15").
    pub image: String,
    /// Ports to expose (container ports).
    #[serde(default)]
    pub ports: Vec<u16>,
    /// Environment variables for the service container.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Readiness probe configuration.
    #[serde(default)]
    pub readiness: Option<YamlReadiness>,
    /// Memory limit (e.g., "512Mi").
    #[serde(default)]
    pub memory: Option<String>,
    /// CPU limit (e.g., "500m").
    #[serde(default)]
    pub cpu: Option<String>,
    /// Override container entrypoint.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Override container args.
    #[serde(default)]
    pub args: Option<Vec<String>>,
}

/// Readiness probe configuration in YAML format.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlReadiness {
    /// Execute a command to check readiness.
    #[serde(default)]
    pub exec: Option<Vec<String>>,
    /// Check if a TCP port is accepting connections.
    #[serde(default)]
    pub tcp: Option<u16>,
    /// HTTP GET health check.
    #[serde(default)]
    pub http: Option<YamlHttpGet>,
    /// Poll interval (e.g., "5s", "2s").
    #[serde(default)]
    pub interval: Option<String>,
    /// Total timeout (e.g., "60s", "30s").
    #[serde(default)]
    pub timeout: Option<String>,
    /// Max retries before giving up.
    #[serde(default)]
    pub retries: Option<u32>,
}

/// HTTP GET readiness check.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlHttpGet {
    /// Port to check.
    pub port: u16,
    /// HTTP path (default: "/").
    #[serde(default = "default_health_path")]
    pub path: String,
}

fn default_health_path() -> String {
    "/".into()
}

/// A single step in a workflow.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlStep {
    /// Step identifier (must be unique within the workflow).
    pub name: String,
    /// Executor type. One of: shell, deno, containerd, buildkit, kubernetes, k8s,
    /// cargo-build, cargo-test, cargo-check, cargo-clippy, cargo-fmt, cargo-doc,
    /// cargo-publish, cargo-audit, cargo-deny, cargo-nextest, cargo-llvm-cov,
    /// cargo-doc-mdx, rust-install, rustup-toolchain, rustup-component,
    /// rustup-target, workflow. Default: "shell".
    #[serde(rename = "type")]
    pub step_type: Option<String>,
    /// Type-specific configuration.
    #[serde(default)]
    pub config: Option<StepConfig>,
    /// Input data references.
    #[serde(default)]
    pub inputs: Vec<DataRef>,
    /// Output data references.
    #[serde(default)]
    pub outputs: Vec<DataRef>,
    /// Steps to run in parallel.
    #[serde(default)]
    pub parallel: Option<Vec<YamlStep>>,
    /// Error handling override for this step.
    #[serde(default)]
    pub error_behavior: Option<YamlErrorBehavior>,
    /// Hook step to run on success.
    #[serde(default)]
    pub on_success: Option<Box<YamlStep>>,
    /// Compensation step to run on failure.
    #[serde(default)]
    pub on_failure: Option<Box<YamlStep>>,
    /// Cleanup step that always runs.
    #[serde(default)]
    pub ensure: Option<Box<YamlStep>>,
    /// Condition that must be true for this step to execute.
    #[serde(default)]
    pub when: Option<YamlCondition>,
}

/// Step configuration (fields are type-specific).
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct StepConfig {
    // --- Shell ---
    /// Shell command to run (shorthand for shell steps).
    pub run: Option<String>,
    /// Path to a script file (deno steps).
    pub file: Option<String>,
    /// Inline script source (deno steps).
    pub script: Option<String>,
    /// Shell binary to use (default: "sh").
    pub shell: Option<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Execution timeout (e.g., "5m", "30s").
    pub timeout: Option<String>,
    /// Working directory.
    pub working_dir: Option<String>,
    /// Artifact inputs to mount: { name: mount_point }.
    #[serde(default)]
    pub inputs: HashMap<String, String>,

    // --- Git ---
    /// Git branch to checkout.
    pub branch: Option<String>,
    /// Git commit SHA to checkout.
    pub commit: Option<String>,
    /// Clone depth for shallow clones.
    pub depth: Option<u32>,
    /// Input key for artifact override.
    pub input: Option<String>,

    // --- Deno ---
    /// Deno sandbox permissions.
    #[serde(default)]
    pub permissions: Option<DenoPermissionsYaml>,
    /// ES modules to import (e.g., "npm:lodash@4").
    #[serde(default)]
    pub modules: Vec<String>,

    // --- BuildKit ---
    /// Dockerfile path.
    pub dockerfile: Option<String>,
    /// Build context path.
    pub context: Option<String>,
    /// Multi-stage build target.
    pub target: Option<String>,
    /// Image tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Build arguments.
    #[serde(default)]
    pub build_args: HashMap<String, String>,
    /// Cache import sources.
    #[serde(default)]
    pub cache_from: Vec<String>,
    /// Cache export destinations.
    #[serde(default)]
    pub cache_to: Vec<String>,
    /// Push built image to registry.
    pub push: Option<bool>,
    /// Output type: image, docker, oci, local, tar.
    pub output_type: Option<String>,
    /// Destination path for non-image exports (e.g. tar file).
    pub output_dest: Option<String>,
    /// BuildKit daemon address.
    pub buildkit_addr: Option<String>,
    /// TLS configuration.
    #[serde(default)]
    pub tls: Option<TlsConfigYaml>,
    /// Registry authentication per registry hostname.
    #[serde(default)]
    pub registry_auth: Option<HashMap<String, RegistryAuthYaml>>,

    // --- Containerd ---
    /// Container image (required for containerd/kubernetes steps).
    pub image: Option<String>,
    /// Container command override.
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Volume mounts.
    #[serde(default)]
    pub volumes: Vec<VolumeMountYaml>,
    /// User:group (e.g., "1000:1000").
    pub user: Option<String>,
    /// Network mode: none, host, bridge.
    pub network: Option<String>,
    /// Memory limit (e.g., "512m").
    pub memory: Option<String>,
    /// CPU limit (e.g., "1.0").
    pub cpu: Option<String>,
    /// Image pull policy: always, if-not-present, never.
    pub pull: Option<String>,
    /// Containerd daemon address.
    pub containerd_addr: Option<String>,
    /// CLI binary name: "nerdctl" (default) or "docker".
    pub cli: Option<String>,

    // --- Kubernetes ---
    /// Kubeconfig path.
    pub kubeconfig: Option<String>,
    /// Namespace override.
    pub namespace: Option<String>,
    /// Image pull policy: Always, IfNotPresent, Never.
    pub pull_policy: Option<String>,

    // --- Cargo ---
    /// Target package for cargo steps (`-p`).
    pub package: Option<String>,
    /// Features to enable.
    #[serde(default)]
    pub features: Vec<String>,
    /// Enable all features.
    #[serde(default)]
    pub all_features: Option<bool>,
    /// Disable default features.
    #[serde(default)]
    pub no_default_features: Option<bool>,
    /// Build in release mode.
    #[serde(default)]
    pub release: Option<bool>,
    /// Build profile (--profile).
    pub profile: Option<String>,
    /// Rust toolchain override (e.g., "nightly").
    pub toolchain: Option<String>,
    /// Additional CLI arguments.
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Output directory for generated files.
    pub output_dir: Option<String>,

    // --- Rustup ---
    /// Components to add (e.g., ["clippy", "rustfmt"]).
    #[serde(default)]
    pub components: Vec<String>,
    /// Compilation targets to add.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Default toolchain for rust-install steps.
    pub default_toolchain: Option<String>,

    // --- Sub-workflow ---
    /// Child workflow ID.
    #[serde(rename = "workflow")]
    pub child_workflow: Option<String>,
    /// Child workflow version.
    #[serde(rename = "workflow_version")]
    pub child_version: Option<u32>,
}

/// Deno sandbox permission configuration.
#[derive(Debug, Deserialize, Serialize, Clone, Default, JsonSchema)]
pub struct DenoPermissionsYaml {
    /// Allowed network hosts.
    #[serde(default)]
    pub net: Vec<String>,
    /// Allowed read paths.
    #[serde(default)]
    pub read: Vec<String>,
    /// Allowed write paths.
    #[serde(default)]
    pub write: Vec<String>,
    /// Allowed environment variable names.
    #[serde(default)]
    pub env: Vec<String>,
    /// Allow subprocess execution.
    #[serde(default)]
    pub run: bool,
    /// Allow dynamic imports.
    #[serde(default)]
    pub dynamic_import: bool,
}

/// Data reference for step inputs/outputs.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DataRef {
    /// Name.
    pub name: String,
    /// Path.
    pub path: Option<String>,
    /// Json path.
    pub json_path: Option<String>,
}

/// TLS configuration for BuildKit connections.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct TlsConfigYaml {
    /// Ca.
    pub ca: Option<String>,
    /// Cert.
    pub cert: Option<String>,
    /// Key.
    pub key: Option<String>,
}

/// Registry authentication credentials.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct RegistryAuthYaml {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
}

/// Volume mount configuration for containerd steps.
#[derive(Debug, Deserialize, Serialize, Clone, JsonSchema)]
pub struct VolumeMountYaml {
    /// Host path.
    pub source: String,
    /// Container path.
    pub target: String,
    /// Mount as read-only.
    #[serde(default)]
    pub readonly: bool,
}

/// Error handling behavior configuration.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct YamlErrorBehavior {
    /// Behavior type: retry, suspend, terminate, compensate.
    #[serde(rename = "type")]
    pub behavior_type: String,
    /// Retry interval (e.g., "60s").
    pub interval: Option<String>,
    /// Maximum retry attempts.
    pub max_retries: Option<u32>,
}

/// Generate the JSON Schema for the WFE workflow YAML format.
pub fn generate_json_schema() -> serde_json::Value {
    let schema = schemars::generate::SchemaSettings::default()
        .into_generator()
        .into_root_schema_for::<YamlWorkflowFile>();
    serde_json::to_value(schema).unwrap_or_default()
}

/// Generate the JSON Schema as YAML for human consumption.
pub fn generate_yaml_schema() -> String {
    let schema = generate_json_schema();
    serde_yaml::to_string(&schema).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_schema_is_non_empty() {
        let schema = generate_json_schema();
        assert!(schema.is_object(), "schema should be a JSON object");
        let obj = schema.as_object().unwrap();
        assert!(
            obj.contains_key("$schema")
                || obj.contains_key("type")
                || obj.contains_key("properties"),
            "schema missing standard JSON Schema keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn yaml_schema_is_non_empty() {
        let yaml = generate_yaml_schema();
        assert!(!yaml.is_empty(), "yaml schema should not be empty");
        assert!(
            yaml.len() > 100,
            "yaml schema suspiciously short: {} chars",
            yaml.len()
        );
    }
}
