use std::collections::HashMap;

use serde::Deserialize;

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
}

#[derive(Debug, Deserialize)]
pub struct DataRef {
    pub name: String,
    pub path: Option<String>,
    pub json_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct YamlErrorBehavior {
    #[serde(rename = "type")]
    pub behavior_type: String,
    pub interval: Option<String>,
    pub max_retries: Option<u32>,
}
