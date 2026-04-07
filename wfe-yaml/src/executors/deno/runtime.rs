use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use deno_core::JsRuntime;
use deno_core::RuntimeOptions;
use wfe_core::WfeError;

use super::config::DenoConfig;
use super::module_loader::WfeModuleLoader;
use super::ops::workflow::{StepMeta, StepOutputs, WorkflowInputs, wfe_ops};
use super::permissions::PermissionChecker;

/// Create a configured `JsRuntime` for executing a workflow step script.
pub fn create_runtime(
    config: &DenoConfig,
    workflow_data: serde_json::Value,
    step_name: &str,
) -> Result<JsRuntime, WfeError> {
    let ext = wfe_ops::init();

    // Build permissions, auto-adding esm.sh if modules are declared.
    let mut permissions = config.permissions.clone();
    if !config.modules.is_empty() && !permissions.net.iter().any(|h| h == "esm.sh") {
        permissions.net.push("esm.sh".to_string());
    }

    let checker = Rc::new(RefCell::new(PermissionChecker::from_config(&permissions)));
    let module_loader = WfeModuleLoader::new(checker.clone());

    let runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![ext],
        module_loader: Some(Rc::new(module_loader)),
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
        state.put(PermissionChecker::from_config(&permissions));
    }

    Ok(runtime)
}

/// Returns whether esm.sh would be auto-added for the given config.
/// Exposed for testing.
pub fn would_auto_add_esm_sh(config: &DenoConfig) -> bool {
    !config.modules.is_empty() && !config.permissions.net.iter().any(|h| h == "esm.sh")
}

#[cfg(test)]
mod tests {
    use super::super::config::DenoPermissions;
    use super::*;

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

    #[test]
    fn auto_add_esm_sh_when_modules_declared() {
        let config = DenoConfig {
            script: Some("1".to_string()),
            file: None,
            permissions: DenoPermissions::default(),
            modules: vec!["npm:lodash@4".to_string()],
            env: HashMap::new(),
            timeout_ms: None,
        };
        assert!(would_auto_add_esm_sh(&config));
    }

    #[test]
    fn no_auto_add_esm_sh_when_no_modules() {
        let config = DenoConfig {
            script: Some("1".to_string()),
            file: None,
            permissions: DenoPermissions::default(),
            modules: vec![],
            env: HashMap::new(),
            timeout_ms: None,
        };
        assert!(!would_auto_add_esm_sh(&config));
    }

    #[test]
    fn no_auto_add_esm_sh_when_already_present() {
        let config = DenoConfig {
            script: Some("1".to_string()),
            file: None,
            permissions: DenoPermissions {
                net: vec!["esm.sh".to_string()],
                ..Default::default()
            },
            modules: vec!["npm:lodash@4".to_string()],
            env: HashMap::new(),
            timeout_ms: None,
        };
        assert!(!would_auto_add_esm_sh(&config));
    }
}
