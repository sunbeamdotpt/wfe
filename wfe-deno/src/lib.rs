//! wfe-deno — Deno/V8 bindings for executing JavaScript/TypeScript workflow steps.
/// Bridge.
pub mod bridge;
/// Ops.
pub mod ops;
/// State.
pub mod state;

use deno_core::{JsRuntime, RuntimeOptions};
use tokio::sync::mpsc;

use crate::ops::wfe_deno_ext;
use crate::state::WfeState;

/// Create a `JsRuntime` with the wfe-deno extension loaded.
///
/// The runtime is ready to execute JavaScript that uses the WFE API:
/// ```js
/// import { WorkflowHost, ExecutionResult } from "ext:wfe-deno/wfe.js";
/// ```
pub fn create_wfe_runtime() -> JsRuntime {
    create_wfe_runtime_with_channel_size(256)
}

/// Create a `JsRuntime` with a custom step-request channel buffer size.
pub fn create_wfe_runtime_with_channel_size(channel_size: usize) -> JsRuntime {
    let ext = wfe_deno_ext::init();

    let runtime = JsRuntime::new(RuntimeOptions {
        extensions: vec![ext],
        ..Default::default()
    });

    let (tx, rx) = mpsc::channel(channel_size);
    {
        let op_state = runtime.op_state();
        let mut op_state = op_state.borrow_mut();
        op_state.put(WfeState::new(tx, rx));
    }

    runtime
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_runtime_succeeds() {
        let _runtime = create_wfe_runtime();
    }

    #[test]
    fn create_runtime_with_custom_channel_size() {
        let _runtime = create_wfe_runtime_with_channel_size(1);
    }

    #[test]
    fn runtime_has_wfe_state() {
        let runtime = create_wfe_runtime();
        let op_state = runtime.op_state();
        let op_state = op_state.borrow();
        let wfe = op_state.borrow::<WfeState>();
        assert!(wfe.host.is_none());
        assert!(wfe.step_request_rx.is_some());
    }
}
