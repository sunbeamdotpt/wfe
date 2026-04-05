use std::time::Duration;

use deno_core::op2;
use deno_core::OpState;
use wfe_core::builder::WorkflowBuilder;
use wfe_core::models::ErrorBehavior;

use crate::state::WfeState;

/// Internal builder state that tracks the WorkflowBuilder and current step.
///
/// We don't use StepBuilder (pub(crate)) — instead we work directly with
/// WorkflowBuilder's public `steps` field, `add_step`, and `wire_outcome`.
pub struct JsBuilderState {
    pub wb: WorkflowBuilder<serde_json::Value>,
    pub current_step: Option<usize>,
}

/// Create a new WorkflowBuilder and return its handle.
#[op2(fast)]
#[smi]
pub fn op_builder_create(state: &mut OpState) -> u32 {
    let wfe = state.borrow_mut::<WfeState>();
    let id = wfe.alloc_builder_id();
    wfe.builders.insert(
        id,
        JsBuilderState {
            wb: WorkflowBuilder::<serde_json::Value>::new(),
            current_step: None,
        },
    );
    id
}

/// Add the first step via `start_with`.
#[op2(fast)]
pub fn op_builder_start_with(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] step_type: String,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;

    if bs.current_step.is_some() {
        return Err(deno_error::JsErrorBox::generic(
            "start_with already called on this builder",
        ));
    }

    let step_id = bs.wb.add_step(&step_type);
    bs.current_step = Some(step_id);
    Ok(())
}

/// Chain the next step via `then`.
#[op2(fast)]
pub fn op_builder_then(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] step_type: String,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;

    let prev_id = bs
        .current_step
        .ok_or_else(|| deno_error::JsErrorBox::generic("call start_with before then"))?;

    let next_id = bs.wb.add_step(&step_type);
    bs.wb.wire_outcome(prev_id, next_id, None);
    bs.current_step = Some(next_id);
    Ok(())
}

/// Set the current step's name.
#[op2(fast)]
pub fn op_builder_name(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] name: String,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;
    let step_id = current_step(bs)?;
    bs.wb.steps[step_id].name = Some(name);
    Ok(())
}

/// Set the current step's JSON config.
#[op2]
pub fn op_builder_config(
    state: &mut OpState,
    #[smi] handle: u32,
    #[serde] config: serde_json::Value,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;
    let step_id = current_step(bs)?;
    bs.wb.steps[step_id].step_config = Some(config);
    Ok(())
}

/// Set the current step's error behavior.
#[op2]
pub fn op_builder_on_error(
    state: &mut OpState,
    #[smi] handle: u32,
    #[serde] behavior: serde_json::Value,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;
    let step_id = current_step(bs)?;
    let eb = parse_error_behavior(&behavior)?;
    bs.wb.steps[step_id].error_behavior = Some(eb);
    Ok(())
}

/// Chain a delay step after the current step.
#[op2(fast)]
pub fn op_builder_delay(
    state: &mut OpState,
    #[smi] handle: u32,
    #[number] ms: u64,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;
    let prev_id = current_step(bs)?;

    let delay_type = std::any::type_name::<wfe_core::primitives::delay::DelayStep>();
    let next_id = bs.wb.add_step(delay_type);
    bs.wb.steps[next_id].step_config = Some(serde_json::json!({
        "duration_millis": ms,
    }));
    bs.wb.wire_outcome(prev_id, next_id, None);
    bs.current_step = Some(next_id);
    Ok(())
}

/// Chain a wait-for-event step after the current step.
#[op2(fast)]
pub fn op_builder_wait_for(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] event_name: String,
    #[string] event_key: String,
) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = get_builder(wfe, handle)?;
    let prev_id = current_step(bs)?;

    let wait_type = std::any::type_name::<wfe_core::primitives::wait_for::WaitForStep>();
    let next_id = bs.wb.add_step(wait_type);
    bs.wb.steps[next_id].step_config = Some(serde_json::json!({
        "event_name": event_name,
        "event_key": event_key,
    }));
    bs.wb.wire_outcome(prev_id, next_id, None);
    bs.current_step = Some(next_id);
    Ok(())
}

/// Build the workflow definition and return it as JSON. Consumes the builder.
#[op2]
#[serde]
pub fn op_builder_build(
    state: &mut OpState,
    #[smi] handle: u32,
    #[string] id: String,
    #[smi] version: u32,
) -> Result<serde_json::Value, deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    let bs = wfe
        .builders
        .remove(&handle)
        .ok_or_else(|| deno_error::JsErrorBox::generic("invalid builder handle"))?;

    let def = bs.wb.build(&id, version);
    serde_json::to_value(&def)
        .map_err(|e| deno_error::JsErrorBox::generic(format!("serialization failed: {e}")))
}

/// Build the workflow definition and register it with the host. Consumes the builder.
#[op2]
pub async fn op_builder_register(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
    #[smi] handle: u32,
    #[string] id: String,
    #[smi] version: u32,
) -> Result<(), deno_error::JsErrorBox> {
    let (def, host) = {
        let mut s = state.borrow_mut();
        let wfe = s.borrow_mut::<WfeState>();
        let bs = wfe
            .builders
            .remove(&handle)
            .ok_or_else(|| deno_error::JsErrorBox::generic("invalid builder handle"))?;
        let def = bs.wb.build(&id, version);
        let host = wfe.host()?.clone();
        (def, host)
    };

    host.register_workflow_definition(def).await;
    Ok(())
}

fn get_builder(
    wfe: &mut WfeState,
    handle: u32,
) -> Result<&mut JsBuilderState, deno_error::JsErrorBox> {
    wfe.builders
        .get_mut(&handle)
        .ok_or_else(|| deno_error::JsErrorBox::generic("invalid builder handle"))
}

fn current_step(bs: &JsBuilderState) -> Result<usize, deno_error::JsErrorBox> {
    bs.current_step
        .ok_or_else(|| deno_error::JsErrorBox::generic("no current step — call start_with first"))
}

fn parse_error_behavior(
    value: &serde_json::Value,
) -> Result<ErrorBehavior, deno_error::JsErrorBox> {
    match value.as_str() {
        Some("suspend") => Ok(ErrorBehavior::Suspend),
        Some("terminate") => Ok(ErrorBehavior::Terminate),
        Some("compensate") => Ok(ErrorBehavior::Compensate),
        _ => {
            if let Some(retry) = value.get("retry") {
                let interval = retry
                    .get("interval")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(60_000);
                let max_retries = retry
                    .get("maxRetries")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3) as u32;
                Ok(ErrorBehavior::Retry {
                    interval: Duration::from_millis(interval),
                    max_retries,
                })
            } else {
                Err(deno_error::JsErrorBox::generic(format!(
                    "invalid error behavior: {value}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parse_suspend_behavior() {
        let eb = parse_error_behavior(&serde_json::json!("suspend")).unwrap();
        assert!(matches!(eb, ErrorBehavior::Suspend));
    }

    #[test]
    fn parse_terminate_behavior() {
        let eb = parse_error_behavior(&serde_json::json!("terminate")).unwrap();
        assert!(matches!(eb, ErrorBehavior::Terminate));
    }

    #[test]
    fn parse_compensate_behavior() {
        let eb = parse_error_behavior(&serde_json::json!("compensate")).unwrap();
        assert!(matches!(eb, ErrorBehavior::Compensate));
    }

    #[test]
    fn parse_retry_behavior() {
        let eb = parse_error_behavior(
            &serde_json::json!({"retry": {"interval": 5000, "maxRetries": 5}}),
        )
        .unwrap();
        match eb {
            ErrorBehavior::Retry {
                interval,
                max_retries,
            } => {
                assert_eq!(interval, Duration::from_millis(5000));
                assert_eq!(max_retries, 5);
            }
            _ => panic!("expected Retry"),
        }
    }

    #[test]
    fn parse_retry_behavior_defaults() {
        let eb = parse_error_behavior(&serde_json::json!({"retry": {}})).unwrap();
        match eb {
            ErrorBehavior::Retry {
                interval,
                max_retries,
            } => {
                assert_eq!(interval, Duration::from_millis(60_000));
                assert_eq!(max_retries, 3);
            }
            _ => panic!("expected Retry"),
        }
    }

    #[test]
    fn parse_invalid_behavior_returns_error() {
        let result = parse_error_behavior(&serde_json::json!("nonsense"));
        assert!(result.is_err());
    }
}
