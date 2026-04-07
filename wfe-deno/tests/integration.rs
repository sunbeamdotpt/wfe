use wfe_deno::create_wfe_runtime;

/// Helper: run a JS script in a fresh wfe runtime and drive the event loop.
async fn run_js(code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = create_wfe_runtime();
    runtime
        .execute_script("<test>", code.to_string())
        .map_err(|e| format!("script error: {e}"))?;
    runtime
        .run_event_loop(Default::default())
        .await
        .map_err(|e| format!("event loop error: {e}"))?;
    Ok(())
}

/// Helper: run a JS module in a fresh wfe runtime and drive the event loop.
async fn run_module(code: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut runtime = create_wfe_runtime();
    let specifier = deno_core::ModuleSpecifier::parse("ext:wfe-deno/test-module.js").unwrap();
    let module_id = runtime
        .load_main_es_module_from_code(&specifier, code.to_string())
        .await
        .map_err(|e| format!("module load error: {e}"))?;
    let eval = runtime.mod_evaluate(module_id);
    runtime
        .run_event_loop(Default::default())
        .await
        .map_err(|e| format!("event loop error: {e}"))?;
    eval.await.map_err(|e| format!("module eval error: {e}"))?;
    Ok(())
}

#[tokio::test]
async fn host_create_via_op() {
    run_js("Deno.core.ops.op_host_create()").await.unwrap();
}

#[tokio::test]
async fn host_create_twice_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        Deno.core.ops.op_host_create();
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn host_start_without_create_fails() {
    let _result = run_js("await Deno.core.ops.op_host_start()").await;
    // Script eval of bare await fails, use module mode
    let result = run_module("await Deno.core.ops.op_host_start()").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn host_create_and_start() {
    run_module(
        r#"
        Deno.core.ops.op_host_create();
        await Deno.core.ops.op_host_start();
        await Deno.core.ops.op_host_stop();
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_create_returns_handle() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        if (typeof h !== "number") throw new Error("expected number handle");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_start_with_and_name() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "my-step");
        Deno.core.ops.op_builder_name(h, "Step One");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_chain_and_build() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "step-a");
        Deno.core.ops.op_builder_name(h, "First");
        Deno.core.ops.op_builder_config(h, { key: "val" });
        Deno.core.ops.op_builder_then(h, "step-b");
        Deno.core.ops.op_builder_name(h, "Second");
        const def = Deno.core.ops.op_builder_build(h, "test-wf", 1);
        if (def.id !== "test-wf") throw new Error("wrong id: " + def.id);
        if (def.version !== 1) throw new Error("wrong version");
        if (def.steps.length !== 2) throw new Error("expected 2 steps, got " + def.steps.length);
        if (def.steps[0].name !== "First") throw new Error("wrong step 0 name");
        if (def.steps[1].name !== "Second") throw new Error("wrong step 1 name");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_config_and_on_error() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "step-a");
        Deno.core.ops.op_builder_config(h, { ns: "ory" });
        Deno.core.ops.op_builder_on_error(h, "suspend");
        const def = Deno.core.ops.op_builder_build(h, "test", 1);
        if (!def.steps[0].step_config) throw new Error("missing step_config");
        if (def.steps[0].step_config.ns !== "ory") throw new Error("wrong config");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_on_error_retry() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "step-a");
        Deno.core.ops.op_builder_on_error(h, { retry: { interval: 1000, maxRetries: 5 } });
        const def = Deno.core.ops.op_builder_build(h, "test", 1);
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_delay_step() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "step-a");
        Deno.core.ops.op_builder_delay(h, 5000);
        const def = Deno.core.ops.op_builder_build(h, "test", 1);
        if (def.steps.length !== 2) throw new Error("expected 2 steps (original + delay)");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_wait_for_step() {
    run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "step-a");
        Deno.core.ops.op_builder_wait_for(h, "order.paid", "order-123");
        const def = Deno.core.ops.op_builder_build(h, "test", 1);
        if (def.steps.length !== 2) throw new Error("expected 2 steps");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_register_with_host() {
    run_module(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "step-a");
        Deno.core.ops.op_builder_name(h, "First");
        await Deno.core.ops.op_builder_register(h, "registered-wf", 1);
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn builder_start_with_twice_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "a");
        Deno.core.ops.op_builder_start_with(h, "b");
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn builder_then_without_start_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_then(h, "b");
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn builder_name_without_step_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_name(h, "oops");
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn builder_invalid_handle_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        Deno.core.ops.op_builder_start_with(999, "a");
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn builder_build_invalid_handle_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        Deno.core.ops.op_builder_build(999, "id", 1);
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn workflow_management_ops() {
    run_module(
        r#"
        Deno.core.ops.op_host_create();
        await Deno.core.ops.op_host_start();

        // Register a no-op step and workflow.
        const h = Deno.core.ops.op_builder_create();
        Deno.core.ops.op_builder_start_with(h, "noop");
        await Deno.core.ops.op_builder_register(h, "mgmt-test", 1);

        // Start a workflow (it will just sit with unknown step type).
        const wfId = await Deno.core.ops.op_start_workflow("mgmt-test", 1, { foo: "bar" });
        if (typeof wfId !== "string") throw new Error("expected string ID");

        // Get the workflow.
        const wf = await Deno.core.ops.op_get_workflow(wfId);
        if (wf.id !== wfId) throw new Error("wrong id");
        if (wf.data.foo !== "bar") throw new Error("wrong data");

        // Suspend.
        const suspended = await Deno.core.ops.op_suspend_workflow(wfId);
        if (!suspended) throw new Error("expected suspend to succeed");

        // Resume.
        const resumed = await Deno.core.ops.op_resume_workflow(wfId);
        if (!resumed) throw new Error("expected resume to succeed");

        // Terminate.
        const terminated = await Deno.core.ops.op_terminate_workflow(wfId);
        if (!terminated) throw new Error("expected terminate to succeed");

        // Suspend after terminate should return false.
        const again = await Deno.core.ops.op_suspend_workflow(wfId);
        if (again) throw new Error("expected suspend after terminate to fail");

        await Deno.core.ops.op_host_stop();
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn step_executor_respond_invalid_id_fails() {
    let result = run_js(
        r#"
        Deno.core.ops.op_host_create();
        Deno.core.ops.op_step_executor_respond(999, { proceed: true }, null);
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn wfe_js_api_execution_result_factories() {
    run_module(
        r#"
        import { ExecutionResult } from "ext:wfe-deno/wfe.js";

        const next = ExecutionResult.next();
        if (!next.proceed) throw new Error("next should proceed");

        const outcome = ExecutionResult.outcome("branch-a");
        if (outcome.outcomeValue !== "branch-a") throw new Error("wrong outcome");

        const persist = ExecutionResult.persist({ page: 2 });
        if (persist.proceed) throw new Error("persist should not proceed");
        if (persist.persistenceData.page !== 2) throw new Error("wrong persist data");

        const sleep = ExecutionResult.sleep(5000);
        if (sleep.sleepFor !== 5000) throw new Error("wrong sleep");

        const wait = ExecutionResult.waitForEvent("ev", "k");
        if (wait.eventName !== "ev") throw new Error("wrong event name");

        const branch = ExecutionResult.branch([1, 2]);
        if (branch.branchValues.length !== 2) throw new Error("wrong branch");

        const output = ExecutionResult.output({ done: true });
        if (!output.proceed) throw new Error("output should proceed");
        if (!output.outputData.done) throw new Error("wrong output");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn wfe_js_api_workflow_host_class() {
    run_module(
        r#"
        import { WorkflowHost } from "ext:wfe-deno/wfe.js";

        const host = WorkflowHost.create();
        await host.start();
        await host.stop();
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn wfe_js_api_workflow_builder_class() {
    run_module(
        r#"
        import { WorkflowHost, WorkflowBuilder } from "ext:wfe-deno/wfe.js";

        const host = WorkflowHost.create();
        const b = new WorkflowBuilder();
        b.startWith("step-a").name("First").config({ key: "val" }).then("step-b").name("Second");
        const def = b.build("builder-test", 1);
        if (def.steps.length !== 2) throw new Error("expected 2 steps");
        if (def.steps[0].name !== "First") throw new Error("wrong name");
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn wfe_js_api_build_workflow_helper() {
    run_module(
        r#"
        import { WorkflowHost } from "ext:wfe-deno/wfe.js";

        const host = WorkflowHost.create();
        await host.buildWorkflow("helper-test", 1, (b) => {
            b.startWith("step-a").name("A").then("step-b").name("B");
        });
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn publish_event_op() {
    run_module(
        r#"
        Deno.core.ops.op_host_create();
        await Deno.core.ops.op_host_start();
        await Deno.core.ops.op_publish_event("test.event", "key-1", { data: true });
        await Deno.core.ops.op_host_stop();
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn start_workflow_without_host_fails() {
    let result = run_module(
        r#"
        await Deno.core.ops.op_start_workflow("nope", 1, {});
    "#,
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn register_step_without_host_fails() {
    let result = run_module(
        r#"
        await Deno.core.ops.op_register_step("nope");
    "#,
    )
    .await;
    assert!(result.is_err());
}
