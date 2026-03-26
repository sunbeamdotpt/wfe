#![cfg(feature = "deno")]

//! End-to-end integration tests for Deno steps in full YAML workflows.
//!
//! These tests compile YAML, register with the WFE host, run to completion,
//! and verify outputs — the same flow a real user would follow.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_yaml::load_single_workflow_from_str;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn run_yaml_workflow(yaml: &str) -> wfe::models::WorkflowInstance {
    run_yaml_workflow_with_data(yaml, serde_json::json!({})).await
}

async fn run_yaml_workflow_with_data(
    yaml: &str,
    data: serde_json::Value,
) -> wfe::models::WorkflowInstance {
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    for (key, factory) in compiled.step_factories {
        host.register_step_factory(&key, factory).await;
    }

    host.register_workflow_definition(compiled.definition.clone())
        .await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        &compiled.definition.id,
        compiled.definition.version,
        data,
        Duration::from_secs(15),
    )
    .await
    .unwrap();

    host.stop().await;
    instance
}

// ---------------------------------------------------------------------------
// Phase 7: Full YAML deno workflow E2E tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_yaml_deno_workflow_basic() {
    let yaml = r#"
workflow:
  id: deno-basic
  version: 1
  steps:
    - name: compute
      type: deno
      config:
        script: |
          const data = inputs();
          output("result", (data.x || 10) + (data.y || 20));
    "#;
    let instance = run_yaml_workflow_with_data(yaml, serde_json::json!({"x": 3, "y": 7})).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    // The deno step output gets merged into workflow data.
    let data = instance.data;
    assert_eq!(data["result"], serde_json::json!(10));
}

#[tokio::test]
async fn full_yaml_deno_workflow_string_output() {
    let yaml = r#"
workflow:
  id: deno-string
  version: 1
  steps:
    - name: greet
      type: deno
      config:
        script: |
          output("message", "hello from deno");
    "#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["message"], serde_json::json!("hello from deno"));
}

#[tokio::test]
async fn yaml_deno_with_fetch_wiremock() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/api/data"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"value": 42})),
        )
        .mount(&server)
        .await;

    let yaml = format!(
        r#"
workflow:
  id: deno-fetch
  version: 1
  steps:
    - name: fetch-step
      type: deno
      config:
        script: |
          const resp = await fetch("{}/api/data");
          const data = await resp.json();
          output("fetched_value", data.value);
        permissions:
          net:
            - "127.0.0.1"
"#,
        server.uri()
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["fetched_value"], serde_json::json!(42));
}

#[tokio::test]
async fn yaml_deno_fetch_post() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/api/submit"))
        .respond_with(wiremock::ResponseTemplate::new(201).set_body_string("accepted"))
        .mount(&server)
        .await;

    let yaml = format!(
        r#"
workflow:
  id: deno-fetch-post
  version: 1
  steps:
    - name: post-step
      type: deno
      config:
        script: |
          const resp = await fetch("{}/api/submit", {{
            method: "POST",
            headers: {{ "content-type": "application/json" }},
            body: JSON.stringify({{ key: "val" }})
          }});
          output("status", resp.status);
          output("body", await resp.text());
        permissions:
          net:
            - "127.0.0.1"
"#,
        server.uri()
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["status"], serde_json::json!(201));
    assert_eq!(instance.data["body"], serde_json::json!("accepted"));
}

#[tokio::test]
async fn yaml_deno_with_permissions_enforced() {
    // Deno step without net permissions should fail when trying to fetch.
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let yaml = format!(
        r#"
workflow:
  id: deno-perms
  version: 1
  steps:
    - name: denied-fetch
      type: deno
      config:
        script: |
          try {{
            await fetch("{}");
            output("result", "should_not_reach");
          }} catch (e) {{
            output("denied", true);
            output("error_msg", e.message || String(e));
          }}
"#,
        server.uri()
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["denied"], serde_json::json!(true));
    let err_msg = instance.data["error_msg"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("Permission denied"),
        "expected permission denied, got: {err_msg}"
    );
}

#[tokio::test]
async fn yaml_mixed_shell_and_deno() {
    let wfe_prefix = "##wfe";
    let yaml = format!(
        r#"
workflow:
  id: mixed-wf
  version: 1
  steps:
    - name: shell-step
      type: shell
      config:
        run: echo "{wfe_prefix}[output greeting=hello]"
    - name: deno-step
      type: deno
      config:
        script: |
          output("computed", 100 + 200);
"#
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Both steps completed.
    let complete_count = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == wfe::models::PointerStatus::Complete)
        .count();
    assert!(
        complete_count >= 2,
        "Expected at least 2 completed pointers, got {complete_count}"
    );

    // Deno output should be present.
    assert_eq!(instance.data["computed"], serde_json::json!(300));
}

#[tokio::test]
async fn yaml_deno_then_shell() {
    let yaml = r#"
workflow:
  id: deno-then-shell
  version: 1
  steps:
    - name: deno-first
      type: deno
      config:
        script: |
          output("from_deno", "js_value");
    - name: shell-second
      type: shell
      config:
        run: echo done
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["from_deno"], serde_json::json!("js_value"));
}

#[tokio::test]
async fn yaml_deno_multiple_steps() {
    let yaml = r#"
workflow:
  id: deno-multi
  version: 1
  steps:
    - name: step-a
      type: deno
      config:
        script: |
          output("a", 1);
    - name: step-b
      type: deno
      config:
        script: |
          output("b", 2);
    - name: step-c
      type: deno
      config:
        script: |
          output("c", 3);
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["a"], serde_json::json!(1));
    assert_eq!(instance.data["b"], serde_json::json!(2));
    assert_eq!(instance.data["c"], serde_json::json!(3));
}

#[tokio::test]
async fn yaml_deno_with_inputs_from_workflow_data() {
    let yaml = r#"
workflow:
  id: deno-inputs
  version: 1
  steps:
    - name: use-inputs
      type: deno
      config:
        script: |
          const data = inputs();
          output("doubled", data.value * 2);
"#;
    let instance =
        run_yaml_workflow_with_data(yaml, serde_json::json!({"value": 21})).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["doubled"], serde_json::json!(42));
}

#[tokio::test]
async fn yaml_deno_with_timeout_in_yaml() {
    let yaml = r#"
workflow:
  id: deno-timeout
  version: 1
  steps:
    - name: quick-step
      type: deno
      config:
        script: |
          output("done", true);
        timeout: "5s"
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["done"], serde_json::json!(true));
}

#[tokio::test]
async fn yaml_deno_with_file_step() {
    // Create a temp JS file.
    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("step.js");
    {
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "output('from_file', 'file_value');").unwrap();
    }

    let yaml = format!(
        r#"
workflow:
  id: deno-file
  version: 1
  steps:
    - name: file-step
      type: deno
      config:
        file: "{}"
"#,
        script_path.to_str().unwrap()
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["from_file"], serde_json::json!("file_value"));
}

#[tokio::test]
async fn yaml_deno_with_modules_import() {
    // Create a temp helper module and a main script that imports it.
    let dir = tempfile::tempdir().unwrap();
    let helper_path = dir.path().join("helper.js");
    {
        let mut f = std::fs::File::create(&helper_path).unwrap();
        writeln!(f, "export function double(x) {{ return x * 2; }}").unwrap();
    }

    // Write the main script to a file too, since inline YAML with import braces is tricky.
    let main_path = dir.path().join("main.js");
    {
        let main_code = format!(
            "import {{ double }} from \"file://{}\";\noutput(\"result\", double(21));",
            helper_path.to_str().unwrap()
        );
        std::fs::write(&main_path, &main_code).unwrap();
    }

    let dir_str = dir.path().to_str().unwrap().to_string();
    let main_path_str = main_path.to_str().unwrap().to_string();
    let yaml = format!(
        r#"
workflow:
  id: deno-module
  version: 1
  steps:
    - name: module-step
      type: deno
      config:
        file: "{main_path_str}"
        permissions:
          read:
            - "{dir_str}"
"#
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["result"], serde_json::json!(42));
}

// ---------------------------------------------------------------------------
// Compiler / Validation integration verification
// ---------------------------------------------------------------------------

#[test]
fn compiler_produces_deno_step_factory() {
    let yaml = r#"
workflow:
  id: compiler-test
  version: 1
  steps:
    - name: js-compute
      type: deno
      config:
        script: "output('x', 1);"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    assert!(!compiled.step_factories.is_empty());
    let (key, _factory) = &compiled.step_factories[0];
    assert!(
        key.contains("deno"),
        "factory key should contain 'deno', got: {key}"
    );
}

#[test]
fn compiler_deno_step_with_permissions() {
    let yaml = r#"
workflow:
  id: perm-test
  version: 1
  steps:
    - name: net-step
      type: deno
      config:
        script: "1;"
        permissions:
          net:
            - "api.example.com"
          read:
            - "/tmp"
          env:
            - "HOME"
          dynamic_import: true
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    assert!(!compiled.step_factories.is_empty());
    // Verify the step config was serialized correctly.
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("net-step"))
        .unwrap();
    let cfg: serde_json::Value = step.step_config.clone().unwrap();
    assert_eq!(cfg["permissions"]["net"][0], "api.example.com");
    assert_eq!(cfg["permissions"]["read"][0], "/tmp");
    assert_eq!(cfg["permissions"]["env"][0], "HOME");
    assert_eq!(cfg["permissions"]["dynamic_import"], true);
}

#[test]
fn compiler_deno_step_with_timeout() {
    let yaml = r#"
workflow:
  id: timeout-test
  version: 1
  steps:
    - name: timed
      type: deno
      config:
        script: "1;"
        timeout: "3s"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("timed"))
        .unwrap();
    let cfg: serde_json::Value = step.step_config.clone().unwrap();
    assert_eq!(cfg["timeout_ms"], serde_json::json!(3000));
}

#[test]
fn compiler_deno_step_with_file() {
    let yaml = r#"
workflow:
  id: file-test
  version: 1
  steps:
    - name: file-step
      type: deno
      config:
        file: "./scripts/run.js"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("file-step"))
        .unwrap();
    let cfg: serde_json::Value = step.step_config.clone().unwrap();
    assert_eq!(cfg["file"], "./scripts/run.js");
}

#[test]
fn validation_rejects_deno_step_no_config() {
    let yaml = r#"
workflow:
  id: bad
  version: 1
  steps:
    - name: no-config
      type: deno
"#;
    let config = HashMap::new();
    let result = load_single_workflow_from_str(yaml, &config);
    match result {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("config") || msg.contains("Deno"),
                "got: {msg}"
            );
        }
        Ok(_) => panic!("expected error for deno step without config"),
    }
}

#[test]
fn validation_rejects_deno_step_no_script_or_file() {
    let yaml = r#"
workflow:
  id: bad
  version: 1
  steps:
    - name: empty-config
      type: deno
      config:
        env:
          FOO: bar
"#;
    let config = HashMap::new();
    let result = load_single_workflow_from_str(yaml, &config);
    match result {
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("script") || msg.contains("file"),
                "got: {msg}"
            );
        }
        Ok(_) => panic!("expected error for deno step without script or file"),
    }
}

#[test]
fn validation_accepts_deno_step_with_script() {
    let yaml = r#"
workflow:
  id: ok
  version: 1
  steps:
    - name: good
      type: deno
      config:
        script: "1+1;"
"#;
    let config = HashMap::new();
    assert!(load_single_workflow_from_str(yaml, &config).is_ok());
}

#[test]
fn validation_accepts_deno_step_with_file() {
    let yaml = r#"
workflow:
  id: ok
  version: 1
  steps:
    - name: good
      type: deno
      config:
        file: "./run.js"
"#;
    let config = HashMap::new();
    assert!(load_single_workflow_from_str(yaml, &config).is_ok());
}

#[test]
fn compiler_mixed_shell_and_deno_produces_both_factories() {
    let yaml = r#"
workflow:
  id: mixed
  version: 1
  steps:
    - name: shell-step
      type: shell
      config:
        run: echo hi
    - name: deno-step
      type: deno
      config:
        script: "output('x', 1);"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    let has_shell = compiled.step_factories.iter().any(|(k, _)| k.contains("shell"));
    let has_deno = compiled.step_factories.iter().any(|(k, _)| k.contains("deno"));
    assert!(has_shell, "should have shell factory");
    assert!(has_deno, "should have deno factory");
}

#[test]
fn compiler_deno_step_with_modules_list() {
    let yaml = r#"
workflow:
  id: mod-test
  version: 1
  steps:
    - name: with-mods
      type: deno
      config:
        script: "1;"
        modules:
          - "npm:lodash@4"
          - "npm:is-number@7"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("with-mods"))
        .unwrap();
    let cfg: serde_json::Value = step.step_config.clone().unwrap();
    let modules = cfg["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 2);
    assert_eq!(modules[0], "npm:lodash@4");
    assert_eq!(modules[1], "npm:is-number@7");
}

#[test]
fn compiler_deno_step_with_env() {
    let yaml = r#"
workflow:
  id: env-test
  version: 1
  steps:
    - name: env-step
      type: deno
      config:
        script: "1;"
        env:
          FOO: bar
          BAZ: qux
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("env-step"))
        .unwrap();
    let cfg: serde_json::Value = step.step_config.clone().unwrap();
    assert_eq!(cfg["env"]["FOO"], "bar");
    assert_eq!(cfg["env"]["BAZ"], "qux");
}

// ---------------------------------------------------------------------------
// Error handling E2E
// ---------------------------------------------------------------------------

#[tokio::test]
async fn yaml_deno_error_propagates() {
    let yaml = r#"
workflow:
  id: deno-error
  version: 1
  error_behavior:
    type: terminate
  steps:
    - name: boom
      type: deno
      config:
        script: |
          throw new Error("kaboom");
      error_behavior:
        type: terminate
"#;
    let instance = run_yaml_workflow(yaml).await;
    // The workflow should terminate because the step fails with terminate error behavior.
    assert_eq!(instance.status, WorkflowStatus::Terminated);
}

#[tokio::test]
async fn yaml_deno_with_on_failure_hook() {
    let yaml = r#"
workflow:
  id: deno-hook
  version: 1
  steps:
    - name: failing-step
      type: deno
      config:
        script: |
          throw new Error("intentional");
      on_failure:
        name: cleanup
        type: deno
        config:
          script: |
            output("cleaned", true);
"#;
    let config = HashMap::new();
    // This should compile without errors.
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    assert!(compiled.step_factories.len() >= 2);
}

#[tokio::test]
async fn yaml_deno_log_does_not_crash() {
    let yaml = r#"
workflow:
  id: deno-log
  version: 1
  steps:
    - name: logging-step
      type: deno
      config:
        script: |
          log("test message from deno");
          output("logged", true);
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["logged"], serde_json::json!(true));
}

#[tokio::test]
async fn yaml_deno_complex_json_output() {
    let yaml = r#"
workflow:
  id: deno-json
  version: 1
  steps:
    - name: json-step
      type: deno
      config:
        script: |
          output("nested", { a: { b: { c: 42 } } });
          output("array", [1, 2, 3]);
          output("null_val", null);
          output("bool_val", false);
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    assert_eq!(instance.data["nested"]["a"]["b"]["c"], serde_json::json!(42));
    assert_eq!(instance.data["array"], serde_json::json!([1, 2, 3]));
    assert!(instance.data["null_val"].is_null());
    assert_eq!(instance.data["bool_val"], serde_json::json!(false));
}

// ---------------------------------------------------------------------------
// Workflow description and error_behavior
// ---------------------------------------------------------------------------

#[test]
fn compiler_deno_workflow_with_description() {
    let yaml = r#"
workflow:
  id: described
  version: 2
  description: "A workflow with deno steps"
  steps:
    - name: js
      type: deno
      config:
        script: "1;"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    assert_eq!(
        compiled.definition.description.as_deref(),
        Some("A workflow with deno steps")
    );
    assert_eq!(compiled.definition.version, 2);
}

#[test]
fn compiler_deno_step_with_error_behavior() {
    let yaml = r#"
workflow:
  id: eb-test
  version: 1
  steps:
    - name: retry-step
      type: deno
      config:
        script: "1;"
      error_behavior:
        type: retry
        max_retries: 5
        interval: "2s"
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("retry-step"))
        .unwrap();
    assert!(step.error_behavior.is_some());
}
