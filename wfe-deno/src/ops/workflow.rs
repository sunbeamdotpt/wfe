use deno_core::op2;
use deno_core::OpState;

use crate::state::WfeState;

/// Start a new workflow instance. Returns the workflow instance ID.
#[op2]
#[string]
pub async fn op_start_workflow(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] definition_id: String,
    #[smi] version: u32,
    #[serde] data: serde_json::Value,
) -> Result<String, deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.start_workflow(&definition_id, version, data)
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("start_workflow failed: {e}")))
}

/// Suspend a running workflow. Returns true if suspension succeeded.
#[op2]
pub async fn op_suspend_workflow(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] id: String,
) -> Result<bool, deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.suspend_workflow(&id)
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("suspend_workflow failed: {e}")))
}

/// Resume a suspended workflow. Returns true if resumption succeeded.
#[op2]
pub async fn op_resume_workflow(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] id: String,
) -> Result<bool, deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.resume_workflow(&id)
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("resume_workflow failed: {e}")))
}

/// Terminate a workflow. Returns true if termination succeeded.
#[op2]
pub async fn op_terminate_workflow(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] id: String,
) -> Result<bool, deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.terminate_workflow(&id)
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("terminate_workflow failed: {e}")))
}

/// Get a workflow instance by ID. Returns the serialized WorkflowInstance.
#[op2]
#[serde]
pub async fn op_get_workflow(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] id: String,
) -> Result<serde_json::Value, deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    let instance = host
        .get_workflow(&id)
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("get_workflow failed: {e}")))?;
    serde_json::to_value(&instance)
        .map_err(|e| deno_error::JsErrorBox::generic(format!("serialization failed: {e}")))
}
