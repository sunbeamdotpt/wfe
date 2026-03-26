use std::collections::HashMap;

use serde::Deserialize;

/// A condition in YAML that determines whether a step executes.
///
/// Uses `#[serde(untagged)]` so serde tries each variant in order.
/// A comparison has a `field:` key; a combinator has `all:/any:/none:/one_of:/not:`.
/// Comparison is listed first because it is more specific (requires `field`).
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum YamlCondition {
    /// Leaf comparison (has a `field:` key).
    Comparison(Box<YamlComparison>),
    /// Combinator with sub-conditions.
    Combinator(YamlCombinator),
}

/// A combinator condition containing sub-conditions.
#[derive(Debug, Deserialize, Clone)]
pub struct YamlCombinator {
    #[serde(default)]
    pub all: Option<Vec<YamlCondition>>,
    #[serde(default)]
    pub any: Option<Vec<YamlCondition>>,
    #[serde(default)]
    pub none: Option<Vec<YamlCondition>>,
    #[serde(default)]
    pub one_of: Option<Vec<YamlCondition>>,
    #[serde(default)]
    pub not: Option<Box<YamlCondition>>,
}

/// A leaf comparison condition that compares a field value.
#[derive(Debug, Deserialize, Clone)]
pub struct YamlComparison {
    pub field: String,
    #[serde(default)]
    pub equals: Option<serde_yaml::Value>,
    #[serde(default)]
    pub not_equals: Option<serde_yaml::Value>,
    #[serde(default)]
    pub gt: Option<serde_yaml::Value>,
    #[serde(default)]
    pub gte: Option<serde_yaml::Value>,
    #[serde(default)]
    pub lt: Option<serde_yaml::Value>,
    #[serde(default)]
    pub lte: Option<serde_yaml::Value>,
    #[serde(default)]
    pub contains: Option<serde_yaml::Value>,
    #[serde(default)]
    pub is_null: Option<bool>,
    #[serde(default)]
    pub is_not_null: Option<bool>,
}

/// Top-level YAML file structure supporting both single and multi-workflow files.
#[derive(Debug, Deserialize)]
pub struct YamlWorkflowFile {
    /// Single workflow (backward compatible).
    pub workflow: Option<WorkflowSpec>,
    /// Multiple workflows in one file.
    pub workflows: Option<Vec<WorkflowSpec>>,
}

/// Legacy single-workflow top-level structure. Kept for backward compatibility
/// with code that deserializes `YamlWorkflow` directly.
#[derive(Debug, Deserialize)]
pub struct YamlWorkflow {
    pub workflow: WorkflowSpec,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowSpec {
    pub id: String,
    pub version: u32,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub error_behavior: Option<YamlErrorBehavior>,
    pub steps: Vec<YamlStep>,
    /// Typed input schema: { field_name: type_string }.
    /// Example: `"repo_url": "string"`, `"tags": "list<string>"`.
    #[serde(default)]
    pub inputs: HashMap<String, String>,
    /// Typed output schema: { field_name: type_string }.
    #[serde(default)]
    pub outputs: HashMap<String, String>,
    /// Allow unknown top-level keys (e.g. `_templates`) for YAML anchors.
    #[serde(flatten)]
    pub _extra: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Deserialize)]
pub struct YamlStep {
    pub name: String,
    #[serde(rename = "type")]
    pub step_type: Option<String>,
    #[serde(default)]
    pub config: Option<StepConfig>,
    #[serde(default)]
    pub inputs: Vec<DataRef>,
    #[serde(default)]
    pub outputs: Vec<DataRef>,
    #[serde(default)]
    pub parallel: Option<Vec<YamlStep>>,
    #[serde(default)]
    pub error_behavior: Option<YamlErrorBehavior>,
    #[serde(default)]
    pub on_success: Option<Box<YamlStep>>,
    #[serde(default)]
    pub on_failure: Option<Box<YamlStep>>,
    #[serde(default)]
    pub ensure: Option<Box<YamlStep>>,
    /// Optional condition that must be true for this step to execute.
    #[serde(default)]
    pub when: Option<YamlCondition>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct StepConfig {
    pub run: Option<String>,
    pub file: Option<String>,
    pub script: Option<String>,
    pub shell: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub timeout: Option<String>,
    pub working_dir: Option<String>,
    #[serde(default)]
    pub permissions: Option<DenoPermissionsYaml>,
    #[serde(default)]
    pub modules: Vec<String>,
    // BuildKit fields
    pub dockerfile: Option<String>,
    pub context: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub build_args: HashMap<String, String>,
    #[serde(default)]
    pub cache_from: Vec<String>,
    #[serde(default)]
    pub cache_to: Vec<String>,
    pub push: Option<bool>,
    pub buildkit_addr: Option<String>,
    #[serde(default)]
    pub tls: Option<TlsConfigYaml>,
    #[serde(default)]
    pub registry_auth: Option<HashMap<String, RegistryAuthYaml>>,
    // Containerd fields
    pub image: Option<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub volumes: Vec<VolumeMountYaml>,
    pub user: Option<String>,
    pub network: Option<String>,
    pub memory: Option<String>,
    pub cpu: Option<String>,
    pub pull: Option<String>,
    pub containerd_addr: Option<String>,
    /// CLI binary name for containerd steps: "nerdctl" (default) or "docker".
    pub cli: Option<String>,
    // Workflow (sub-workflow) fields
    /// Child workflow ID (for `type: workflow` steps).
    #[serde(rename = "workflow")]
    pub child_workflow: Option<String>,
    /// Child workflow version (for `type: workflow` steps).
    #[serde(rename = "workflow_version")]
    pub child_version: Option<u32>,
}

/// YAML-level permission configuration for Deno steps.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct DenoPermissionsYaml {
    #[serde(default)]
    pub net: Vec<String>,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub run: bool,
    #[serde(default)]
    pub dynamic_import: bool,
}

#[derive(Debug, Deserialize)]
pub struct DataRef {
    pub name: String,
    pub path: Option<String>,
    pub json_path: Option<String>,
}

/// YAML-level TLS configuration for BuildKit steps.
#[derive(Debug, Deserialize, Clone)]
pub struct TlsConfigYaml {
    pub ca: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
}

/// YAML-level registry auth configuration for BuildKit steps.
#[derive(Debug, Deserialize, Clone)]
pub struct RegistryAuthYaml {
    pub username: String,
    pub password: String,
}

/// YAML-level volume mount configuration for containerd steps.
#[derive(Debug, Deserialize, Clone)]
pub struct VolumeMountYaml {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub readonly: bool,
}

#[derive(Debug, Deserialize)]
pub struct YamlErrorBehavior {
    #[serde(rename = "type")]
    pub behavior_type: String,
    pub interval: Option<String>,
    pub max_retries: Option<u32>,
}
