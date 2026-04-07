use deno_core::OpState;
use deno_core::op2;

use crate::state::WfeState;

/// Publish an event to the workflow host for matching subscriptions.
#[op2]
pub async fn op_publish_event(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[string] event_name: String,
    #[string] event_key: String,
    #[serde] data: serde_json::Value,
) -> Result<(), deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.publish_event(&event_name, &event_key, data)
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("publish_event failed: {e}")))
}
