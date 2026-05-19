#![cfg(feature = "deno")]

use std::collections::HashMap;
use std::io::Write;

use wfe_yaml::executors::deno::config::{DenoConfig, DenoPermissions};
use wfe_yaml::executors::deno::module_loader::WfeModuleLoader;
use wfe_yaml::executors::deno::permissions::PermissionChecker;
use wfe_yaml::executors::deno::runtime::{create_runtime, would_auto_add_esm_sh};
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
        definition: None,
        item: None,
        execution_pointer: pointer,
        persistence_data: None,
        step,
        workflow,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
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
    let result = wfe_yaml::load_single_workflow_from_str(yaml, &config);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("config") || msg.contains("Deno"), "got: {msg}");
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
    let result = wfe_yaml::load_single_workflow_from_str(yaml, &config);
    assert!(result.is_err());
    let msg = result.err().unwrap().to_string();
    assert!(msg.contains("script") || msg.contains("file"), "got: {msg}");
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
    let compiled = wfe_yaml::load_single_workflow_from_str(yaml, &config).unwrap();
    assert!(!compiled.step_factories.is_empty());
    let (key, _factory) = &compiled.step_factories[0];
    assert!(
        key.contains("deno"),
        "factory key should contain 'deno', got: {key}"
    );
}

// ---------------------------------------------------------------------------
// Phase 4: HTTP op integration tests
// ---------------------------------------------------------------------------

fn config_with_net(script: &str, net_hosts: &[&str]) -> DenoConfig {
    DenoConfig {
        script: Some(script.to_string()),
        file: None,
        permissions: DenoPermissions {
            net: net_hosts.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        },
        modules: vec![],
        env: HashMap::new(),
        timeout_ms: Some(10000),
    }
}

#[tokio::test]
async fn deno_fetch_allowed_host() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        const resp = await fetch("{}");
        output("status", resp.status);
        output("body", await resp.text());
        "#,
        server.uri()
    );
    let mut step = DenoStep::new(config_with_net(&script, &["127.0.0.1"]));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["status"], serde_json::json!(200));
    assert_eq!(data["body"], serde_json::json!("ok"));
}

#[tokio::test]
async fn deno_fetch_denied_host() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    // Do NOT add 127.0.0.1 to net permissions.
    let script = format!(
        r#"
        try {{
            await fetch("{}");
            output("result", "should_not_reach");
        }} catch (e) {{
            output("error", e.message || String(e));
        }}
        "#,
        server.uri()
    );
    let mut step = DenoStep::new(config_with_net(&script, &[]));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert!(
        data.get("error").is_some(),
        "expected permission error, got: {data:?}"
    );
    let err_msg = data["error"].as_str().unwrap();
    assert!(err_msg.contains("Permission denied"), "got: {err_msg}");
}

#[tokio::test]
async fn deno_fetch_returns_json() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"name": "alice", "age": 30})),
        )
        .mount(&server)
        .await;

    let script = format!(
        r#"
        const resp = await fetch("{}");
        const data = await resp.json();
        output("name", data.name);
        output("age", data.age);
        "#,
        server.uri()
    );
    let mut step = DenoStep::new(config_with_net(&script, &["127.0.0.1"]));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["name"], serde_json::json!("alice"));
    assert_eq!(data["age"], serde_json::json!(30));
}

#[tokio::test]
async fn deno_fetch_post_with_body() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::header(
            "content-type",
            "application/json",
        ))
        .and(wiremock::matchers::body_json(
            serde_json::json!({"key": "val"}),
        ))
        .respond_with(wiremock::ResponseTemplate::new(201).set_body_string("created"))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        const resp = await fetch("{}", {{
            method: "POST",
            headers: {{ "content-type": "application/json" }},
            body: JSON.stringify({{ key: "val" }})
        }});
        output("status", resp.status);
        output("body", await resp.text());
        "#,
        server.uri()
    );
    let mut step = DenoStep::new(config_with_net(&script, &["127.0.0.1"]));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["status"], serde_json::json!(201));
    assert_eq!(data["body"], serde_json::json!("created"));
}

#[tokio::test]
async fn deno_fetch_no_permissions_denies_all() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&server)
        .await;

    // Empty net allowlist should deny everything.
    let script = format!(
        r#"
        try {{
            await fetch("{}");
            output("result", "should_not_reach");
        }} catch (e) {{
            output("denied", true);
        }}
        "#,
        server.uri()
    );
    let mut step = DenoStep::new(config_with_net(&script, &[]));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["denied"], serde_json::json!(true));
}

// ---------------------------------------------------------------------------
// Phase 5: Module loader integration tests
// ---------------------------------------------------------------------------

#[test]
fn deno_module_loader_resolves_npm_to_esm_sh() {
    let result = WfeModuleLoader::resolve_npm_specifier("npm:lodash@4.17.21");
    assert_eq!(result, Some("https://esm.sh/lodash@4.17.21".to_string()));

    let scoped = WfeModuleLoader::resolve_npm_specifier("npm:@scope/pkg@1.0.0");
    assert_eq!(scoped, Some("https://esm.sh/@scope/pkg@1.0.0".to_string()));

    // Non-npm specifiers return None.
    assert!(WfeModuleLoader::resolve_npm_specifier("https://cdn.com/mod.js").is_none());
    assert!(WfeModuleLoader::resolve_npm_specifier("./local.js").is_none());
}

#[test]
fn deno_esm_sh_auto_added_to_net_allowlist() {
    // When modules are declared, esm.sh should be auto-added.
    let config = DenoConfig {
        script: Some("1".to_string()),
        file: None,
        permissions: DenoPermissions::default(),
        modules: vec!["npm:lodash@4".to_string()],
        env: HashMap::new(),
        timeout_ms: None,
    };
    assert!(would_auto_add_esm_sh(&config));

    // When no modules, esm.sh is not added.
    let config_no_modules = DenoConfig {
        script: Some("1".to_string()),
        file: None,
        permissions: DenoPermissions::default(),
        modules: vec![],
        env: HashMap::new(),
        timeout_ms: None,
    };
    assert!(!would_auto_add_esm_sh(&config_no_modules));

    // When esm.sh already present, not duplicated.
    let config_already = DenoConfig {
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
    assert!(!would_auto_add_esm_sh(&config_already));
}

#[tokio::test]
async fn deno_import_local_file() {
    // Create a temp JS file to import.
    let dir = tempfile::tempdir().unwrap();
    let helper_path = dir.path().join("helper.js");
    let mut f = std::fs::File::create(&helper_path).unwrap();
    writeln!(
        f,
        "export function greet(name) {{ return `hello ${{name}}`; }}"
    )
    .unwrap();
    drop(f);

    let main_path = dir.path().join("main.js");
    let main_code = format!(
        r#"import {{ greet }} from "file://{}";
output("greeting", greet("world"));"#,
        helper_path.to_str().unwrap()
    );
    std::fs::write(&main_path, &main_code).unwrap();

    let config = DenoConfig {
        script: None,
        file: Some(main_path.to_str().unwrap().to_string()),
        permissions: DenoPermissions {
            read: vec![dir.path().to_str().unwrap().to_string()],
            ..Default::default()
        },
        modules: vec![],
        env: HashMap::new(),
        timeout_ms: Some(5000),
    };

    let mut step = DenoStep::new(config);
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["greeting"], serde_json::json!("hello world"));
}

#[tokio::test]
async fn deno_dynamic_import_denied() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("export const x = 1;"))
        .mount(&server)
        .await;

    // dynamic_import defaults to false.
    let script = format!(
        r#"
        try {{
            await import("{}/mod.js");
            output("result", "should_not_reach");
        }} catch (e) {{
            output("error", e.message || String(e));
        }}
        "#,
        server.uri()
    );
    let mut step = DenoStep::new(config_with_net(&script, &["127.0.0.1"]));
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert!(
        data.get("error").is_some(),
        "expected dynamic import to be denied, got: {data:?}"
    );
    let err_msg = data["error"].as_str().unwrap();
    assert!(
        err_msg.contains("Dynamic import") || err_msg.contains("not allowed"),
        "expected dynamic import denied error, got: {err_msg}"
    );
}

#[tokio::test]
async fn deno_import_npm_module() {
    // Use a tiny npm package via esm.sh.
    let script = r#"
        import isNumber from "npm:is-number@7.0.0";
        output("result", isNumber(42));
    "#;
    let config = DenoConfig {
        script: Some(script.to_string()),
        file: None,
        permissions: DenoPermissions {
            net: vec!["esm.sh".to_string()],
            ..Default::default()
        },
        modules: vec!["npm:is-number@7.0.0".to_string()],
        env: HashMap::new(),
        timeout_ms: Some(30000),
    };

    let mut step = DenoStep::new(config);
    let (ws, wf, ptr) = make_test_fixtures(serde_json::json!({}));
    let ctx = make_context(&ws, &wf, &ptr);
    let result = step.run(&ctx).await.unwrap();
    let data = result.output_data.unwrap();
    assert_eq!(data["result"], serde_json::json!(true));
}
