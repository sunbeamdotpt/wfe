use std::sync::Arc;

use deno_core::op2;
use deno_core::OpState;

use crate::bridge::JsStepBody;
use crate::state::WfeState;

/// Register a JS step type with the host.
///
/// On the Rust side this creates a factory that produces `JsStepBody` instances.
/// On the JS side the caller also registers the actual function via
/// `__wfe_registerStepFunction(stepType, fn)`.
#[op2]
pub async fn op_register_step(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] step_type: String,
) -> Result<(), deno_error::JsErrorBox> {
    let (host, tx) = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        (wfe.host()?.clone(), wfe.step_request_tx.clone())
    };
    let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));

    host.register_step_factory(
        &step_type,
        move || Box::new(JsStepBody::new(tx.clone(), counter.clone())),
    )
    .await;

    Ok(())
}

/// Poll for a step execution request from the Rust executor.
///
/// This is an async op that blocks until a step needs to be executed.
/// Returns `{ requestId, stepType, context }` or null on shutdown.
#[op2]
#[serde]
pub async fn op_step_executor_poll(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
) -> Result<serde_json::Value, deno_error::JsErrorBox> {
    // Take the receiver out of state (only the first call gets it).
    let mut rx = {
        let mut s = state.borrow_mut();
        let wfe = s.borrow_mut::<WfeState>();
        match wfe.step_request_rx.take() {
            Some(rx) => rx,
            None => {
                return Err(deno_error::JsErrorBox::generic(
                    "step executor poll already active",
                ));
            }
        }
    };

    // Wait for the next request.
    match rx.recv().await {
        Some(req) => {
            let request_id = req.request_id;
            let result = serde_json::json!({
                "requestId": request_id,
                "stepType": req.step_type,
                "context": req.context,
            });

            // Store the response sender for op_step_executor_respond.
            {
                let mut s = state.borrow_mut();
                let wfe = s.borrow_mut::<WfeState>();
                wfe.inflight.insert(request_id, req.response_tx);
                // Put the receiver back so we can poll again.
                wfe.step_request_rx = Some(rx);
            }

            Ok(result)
        }
        None => {
            // Channel closed — shutdown.
            Ok(serde_json::Value::Null)
        }
    }
}

/// Send a step execution result back to the Rust executor.
#[op2]
pub fn op_step_executor_respond(
    state: &mut OpState,
    #[smi] request_id: u32,
    #[serde] result: Option<serde_json::Value>,
    #[string] error: Option<String>,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let tx = wfe.inflight.remove(&request_id).ok_or_else(|| {
        deno_error::JsErrorBox::generic(format!("no inflight request with id {request_id}"))
    })?;

    let response = match error {
        Some(err) => Err(err),
        None => Ok(result.unwrap_or(serde_json::json!({"proceed": true}))),
    };

    // Ignore send failure — the executor may have timed out.
    let _ = tx.send(response);
    Ok(())
}
