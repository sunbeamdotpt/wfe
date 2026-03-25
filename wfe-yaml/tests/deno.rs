#![cfg(feature = "deno")]

use std::collections::HashMap;

use wfe_yaml::executors::deno::config::{DenoConfig, DenoPermissions};
use wfe_yaml::executors::deno::permissions::PermissionChecker;
use wfe_yaml::executors::deno::runtime::create_runtime;
use wfe_yaml::executors::deno::step::DenoStep;

use wfe_core::models::execution_pointer::ExecutionPointer;
use wfe_core::models::workflow_definition::WorkflowStep;
use wfe_core::models::workflow_instance::WorkflowInstance;
use wfe_core::traits::step::{StepBody, StepExecutionContext};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_config(script: &str) -> DenoConfig {
    DenoConfig {
        script: Some(script.to_string()),
        file: None,
        permissions: DenoPermissions::default(),
        modules: vec![],
        env: HashMap::new(),
        timeout_ms: None,
    }
}

fn make_context<'a>(
    step: &'a WorkflowStep,
    workflow: &'a WorkflowInstance,
    pointer: &'a ExecutionPointer,
) -> StepExecutionContext<'a> {
    StepExecutionContext {
        item: None,
        execution_pointer: pointer,
        persistence_data: None,
        step,
        workflow,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
    }
}

fn make_test_fixtures(
    data: serde_json::Value,
) -> (WorkflowStep, WorkflowInstance, ExecutionPointer) {
    let mut step = WorkflowStep::new(0, "deno");
    step.name = Some("test-deno-step".to_string());
    let workflow = WorkflowInstance::new("test-workflow", 1, data);
    let pointer = ExecutionPointer::new(0);
    (step, workflow, pointer)
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deno_inline_script_completes() {
    let mut step = DenoStep::new(default_config("1 + 1;"));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);
}

#[tokio::test]
async fn deno_output_captured() {
    let mut step = DenoStep::new(default_config(r#"output("greeting", "hello");"#));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);
    let data = result.output_data.unwrap();
    assert_eq!(data["greeting"], serde_json::json!("hello"));
}

#[tokio::test]
async fn deno_inputs_available() {
    let script = r#"
        const data = inputs();
        output("got_name", data.name);
    "#;
    let mut step = DenoStep::new(default_config(script));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({"name": "alice"}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["got_name"], serde_json::json!("alice"));
}

#[tokio::test]
async fn deno_script_error_fails_step() {
    let mut step = DenoStep::new(default_config("throw new Error('boom');"));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let err = step.run(&ctx).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("boom") || msg.contains("Error"), "got: {msg}");
}

#[tokio::test]
async fn deno_timeout_kills_execution() {
    let mut config = default_config("while (true) {}");
    config.timeout_ms = Some(100);
    let mut step = DenoStep::new(config);
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let err = step.run(&ctx).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("timed out"), "got: {msg}");
}

#[tokio::test]
async fn deno_multiple_outputs() {
    let script = r#"
        output("a", 1);
        output("b", "two");
        output("c", true);
    "#;
    let mut step = DenoStep::new(default_config(script));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["a"], serde_json::json!(1));
    assert_eq!(data["b"], serde_json::json!("two"));
    assert_eq!(data["c"], serde_json::json!(true));
}

#[tokio::test]
async fn deno_log_works() {
    let mut step = DenoStep::new(default_config(r#"log("hello from deno");"#));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    // Should not crash.
    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);
}

#[tokio::test]
async fn deno_empty_script_completes() {
    let mut step = DenoStep::new(default_config(""));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);
    assert!(result.output_data.is_none());
}

// ---------------------------------------------------------------------------
// Permission integration tests
// ---------------------------------------------------------------------------

#[test]
fn permission_net_allowed() {
    let perms = DenoPermissions {
        net: vec!["api.example.com".to_string()],
        ..Default::default()
    };
    let checker = PermissionChecker::from_config(&perms);
    assert!(checker.check_net("api.example.com").is_ok());
}

#[test]
fn permission_net_denied() {
    let perms = DenoPermissions::default();
    let checker = PermissionChecker::from_config(&perms);
    assert!(checker.check_net("evil.com").is_err());
}

#[test]
fn permission_env_denied() {
    let perms = DenoPermissions {
        env: vec!["SAFE_VAR".to_string()],
        ..Default::default()
    };
    let checker = PermissionChecker::from_config(&perms);
    assert!(checker.check_env("SECRET_KEY").is_err());
    assert!(checker.check_env("SAFE_VAR").is_ok());
}

// ---------------------------------------------------------------------------
// Runtime creation tests
// ---------------------------------------------------------------------------

#[test]
fn runtime_creation_succeeds() {
    let config = default_config("1");
    let rt = create_runtime(&config, serde_json::json!({"x": 1}), "test");
    assert!(rt.is_ok());
}

#[test]
fn runtime_op_state_populated() {
    let config = default_config("1");
    let rt = create_runtime(&config, serde_json::json!({"foo": "bar"}), "my-step").unwrap();
    let state = rt.op_state();
    let state = state.borrow();
    let inputs = state.borrow::<wfe_yaml::executors::deno::ops::WorkflowInputs>();
    assert_eq!(inputs.data, serde_json::json!({"foo": "bar"}));
}

// ---------------------------------------------------------------------------
// Validation tests
// ---------------------------------------------------------------------------

#[test]
fn validation_deno_step_missing_config() {
    let yaml = r#"
workflow:
  id: test
  version: 1
  steps:
    - name: run_js
      type: deno
"#;
    let config = HashMap::new();
    let result = wfe_yaml::load_workflow_from_str(yaml, &config);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("config") || msg.contains("Deno"),
        "got: {msg}"
    );
}

#[test]
fn validation_deno_step_missing_script_and_file() {
    let yaml = r#"
workflow:
  id: test
  version: 1
  steps:
    - name: run_js
      type: deno
      config:
        env:
          FOO: bar
"#;
    let config = HashMap::new();
    let result = wfe_yaml::load_workflow_from_str(yaml, &config);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("script") || msg.contains("file"),
        "got: {msg}"
    );
}

#[test]
fn compilation_deno_step_produces_factory() {
    let yaml = r#"
workflow:
  id: test-deno
  version: 1
  steps:
    - name: js_step
      type: deno
      config:
        script: "output('key', 'val');"
"#;
    let config = HashMap::new();
    let compiled = wfe_yaml::load_workflow_from_str(yaml, &config).unwrap();
    assert!(!compiled.step_factories.is_empty());
    let (key, _factory) = &compiled.step_factories[0];
    assert!(key.contains("deno"), "factory key should contain 'deno', got: {key}");
}
