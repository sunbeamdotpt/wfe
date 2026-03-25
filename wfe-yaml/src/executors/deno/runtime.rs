use std::collections::HashMap;

use deno_core::JsRuntime;
use deno_core::RuntimeOptions;
use wfe_core::WfeError;

use super::config::DenoConfig;
use super::ops::workflow::{wfe_ops, StepMeta, StepOutputs, WorkflowInputs};
use super::permissions::PermissionChecker;

/// Create a configured `JsRuntime` for executing a workflow step script.
pub fn create_runtime(
    config: &DenoConfig,
    workflow_data: serde_json::Value,
    step_name: &str,
) -> Result<JsRuntime, WfeError> {
    let ext = wfe_ops::init();

    let runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![ext],
        ..Default::default()
    });

    // Populate OpState with our workflow types.
    {
        let state = runtime.op_state();
        let mut state = state.borrow_mut();
        state.put(WorkflowInputs {
            data: workflow_data,
        });
        state.put(StepOutputs {
            map: HashMap::new(),
        });
        state.put(StepMeta {
            name: step_name.to_string(),
        });
        state.put(PermissionChecker::from_config(&config.permissions));
    }

    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::DenoPermissions;

    #[test]
    fn create_runtime_succeeds() {
        let config = DenoConfig {
            script: Some("1+1".to_string()),
            file: None,
            permissions: DenoPermissions::default(),
            modules: vec![],
            env: HashMap::new(),
            timeout_ms: None,
        };
        let runtime = create_runtime(&config, serde_json::json!({}), "test-step");
        assert!(runtime.is_ok());
    }

    #[test]
    fn create_runtime_has_op_state() {
        let config = DenoConfig {
            script: None,
            file: None,
            permissions: DenoPermissions::default(),
            modules: vec![],
            env: HashMap::new(),
            timeout_ms: None,
        };
        let runtime =
            create_runtime(&config, serde_json::json!({"key": "val"}), "my-step").unwrap();
        let state = runtime.op_state();
        let state = state.borrow();
        let inputs = state.borrow::<WorkflowInputs>();
        assert_eq!(inputs.data, serde_json::json!({"key": "val"}));
        let meta = state.borrow::<StepMeta>();
        assert_eq!(meta.name, "my-step");
    }
}
