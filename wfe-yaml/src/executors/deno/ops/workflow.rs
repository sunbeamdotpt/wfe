use std::collections::HashMap;

use deno_core::op2;
use deno_core::OpState;

/// Workflow data available to the script via `inputs()`.
pub struct WorkflowInputs {
    pub data: serde_json::Value,
}

/// Accumulates key/value outputs set by the script via `output(key, value)`.
pub struct StepOutputs {
    pub map: HashMap<String, serde_json::Value>,
}

/// Metadata about the currently executing step.
pub struct StepMeta {
    pub name: String,
}

/// Returns the workflow input data to JavaScript.
#[op2]
#[serde]
pub fn op_inputs(state: &mut OpState) -> serde_json::Value {
    let inputs = state.borrow::<WorkflowInputs>();
    inputs.data.clone()
}

/// Stores a key/value pair in the step outputs.
#[op2]
pub fn op_output(
    state: &mut OpState,
    #[string] key: String,
    #[serde] value: serde_json::Value,
) {
    let outputs = state.borrow_mut::<StepOutputs>();
    outputs.map.insert(key, value);
}

/// Logs a message via the tracing crate.
#[op2(fast)]
pub fn op_log(state: &mut OpState, #[string] msg: String) {
    let name = state.borrow::<StepMeta>().name.clone();
    tracing::info!(step = %name, "{}", msg);
}

deno_core::extension!(
    wfe_ops,
    ops = [op_inputs, op_output, op_log],
    esm_entry_point = "ext:wfe/bootstrap.js",
    esm = ["ext:wfe/bootstrap.js" = "src/executors/deno/js/bootstrap.js"],
);
