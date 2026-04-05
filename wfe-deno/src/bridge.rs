use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use wfe_core::models::ExecutionResult;
use wfe_core::traits::step::{StepBody, StepExecutionContext};
use wfe_core::WfeError;

/// A request sent from the executor (tokio) to the V8 thread.
pub struct StepRequest {
    pub request_id: u32,
    pub step_type: String,
    pub context: serde_json::Value,
    pub response_tx: oneshot::Sender<Result<serde_json::Value, String>>,
}

/// A `StepBody` implementation that bridges to JavaScript via channels.
///
/// When the workflow executor calls `run()`, this sends a serialized
/// `StepExecutionContext` to the V8 thread and awaits the response.
pub struct JsStepBody {
    request_tx: mpsc::Sender<StepRequest>,
    request_id_counter: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl JsStepBody {
    pub fn new(
        request_tx: mpsc::Sender<StepRequest>,
        request_id_counter: std::sync::Arc<std::sync::atomic::AtomicU32>,
    ) -> Self {
        Self {
            request_tx,
            request_id_counter,
        }
    }
}

#[async_trait]
impl StepBody for JsStepBody {
    async fn run(
        &mut self,
        context: &StepExecutionContext<'_>,
    ) -> wfe_core::Result<ExecutionResult> {
        let ctx_json = serialize_context(context);
        let (tx, rx) = oneshot::channel();
        let request_id = self
            .request_id_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        self.request_tx
            .send(StepRequest {
                request_id,
                step_type: context.step.step_type.clone(),
                context: ctx_json,
                response_tx: tx,
            })
            .await
            .map_err(|_| WfeError::StepExecution("step request channel closed".into()))?;

        let result_json = rx
            .await
            .map_err(|_| WfeError::StepExecution("step response channel dropped".into()))?
            .map_err(WfeError::StepExecution)?;

        deserialize_execution_result(&result_json)
    }
}

/// Serializable view of `StepExecutionContext` for passing to JavaScript.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsStepContext {
    pub item: Option<serde_json::Value>,
    pub persistence_data: Option<serde_json::Value>,
    pub step: JsStepInfo,
    pub workflow: JsWorkflowInfo,
    pub pointer: JsPointerInfo,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsStepInfo {
    pub id: usize,
    pub name: Option<String>,
    pub step_type: String,
    pub step_config: Option<serde_json::Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsWorkflowInfo {
    pub id: String,
    pub definition_id: String,
    pub version: u32,
    pub status: String,
    pub data: serde_json::Value,
    pub create_time: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsPointerInfo {
    pub id: String,
    pub step_id: usize,
    pub step_name: Option<String>,
    pub retry_count: u32,
}

/// Serialize a `StepExecutionContext` into a JSON value for JavaScript.
pub fn serialize_context(ctx: &StepExecutionContext<'_>) -> serde_json::Value {
    let js_ctx = JsStepContext {
        item: ctx.item.cloned(),
        persistence_data: ctx.persistence_data.cloned(),
        step: JsStepInfo {
            id: ctx.step.id,
            name: ctx.step.name.clone(),
            step_type: ctx.step.step_type.clone(),
            step_config: ctx.step.step_config.clone(),
        },
        workflow: JsWorkflowInfo {
            id: ctx.workflow.id.clone(),
            definition_id: ctx.workflow.workflow_definition_id.clone(),
            version: ctx.workflow.version,
            status: format!("{:?}", ctx.workflow.status),
            data: ctx.workflow.data.clone(),
            create_time: ctx.workflow.create_time,
        },
        pointer: JsPointerInfo {
            id: ctx.execution_pointer.id.clone(),
            step_id: ctx.execution_pointer.step_id,
            step_name: ctx.execution_pointer.step_name.clone(),
            retry_count: ctx.execution_pointer.retry_count,
        },
    };
    serde_json::to_value(js_ctx).unwrap_or(serde_json::Value::Null)
}

/// Shape of an `ExecutionResult` as returned from JavaScript.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JsExecutionResult {
    #[serde(default = "default_true")]
    pub proceed: bool,
    pub outcome_value: Option<serde_json::Value>,
    pub sleep_for: Option<u64>,
    pub persistence_data: Option<serde_json::Value>,
    pub event_name: Option<String>,
    pub event_key: Option<String>,
    pub branch_values: Option<Vec<serde_json::Value>>,
    pub output_data: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

/// Deserialize a JavaScript execution result into the Rust `ExecutionResult`.
pub fn deserialize_execution_result(
    value: &serde_json::Value,
) -> wfe_core::Result<ExecutionResult> {
    let js_result: JsExecutionResult = serde_json::from_value(value.clone()).map_err(|e| {
        WfeError::StepExecution(format!("failed to deserialize ExecutionResult from JS: {e}"))
    })?;

    Ok(ExecutionResult {
        proceed: js_result.proceed,
        outcome_value: js_result.outcome_value,
        sleep_for: js_result.sleep_for.map(Duration::from_millis),
        persistence_data: js_result.persistence_data,
        event_name: js_result.event_name,
        event_key: js_result.event_key,
        event_as_of: None,
        branch_values: js_result.branch_values,
        poll_endpoint: None,
        output_data: js_result.output_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use wfe_core::models::{ExecutionPointer, WorkflowInstance, WorkflowStatus, WorkflowStep};

    fn make_test_context() -> (WorkflowInstance, WorkflowStep, ExecutionPointer) {
        let instance = WorkflowInstance {
            id: "wf-1".into(),
            workflow_definition_id: "test-def".into(),
            version: 1,
            description: None,
            reference: None,
            execution_pointers: vec![],
            next_execution: None,
            status: WorkflowStatus::Runnable,
            data: serde_json::json!({"name": "World"}),
            create_time: Utc::now(),
            complete_time: None,
        };
        let step = WorkflowStep::new(0, "MyStep");
        let pointer = ExecutionPointer::new(0);
        (instance, step, pointer)
    }

    #[test]
    fn serialize_context_produces_valid_json() {
        let (instance, mut step, pointer) = make_test_context();
        step.name = Some("greet".into());
        step.step_config = Some(serde_json::json!({"key": "val"}));

        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
        };

        let json = serialize_context(&ctx);
        assert_eq!(json["step"]["name"], "greet");
        assert_eq!(json["step"]["stepConfig"]["key"], "val");
        assert_eq!(json["workflow"]["data"]["name"], "World");
        assert_eq!(json["workflow"]["definitionId"], "test-def");
        assert_eq!(json["pointer"]["stepId"], 0);
    }

    #[test]
    fn serialize_context_with_item() {
        let (instance, step, pointer) = make_test_context();
        let item = serde_json::json!({"id": 42});

        let ctx = StepExecutionContext {
            item: Some(&item),
            execution_pointer: &pointer,
            persistence_data: Some(&serde_json::json!({"saved": true})),
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
        };

        let json = serialize_context(&ctx);
        assert_eq!(json["item"]["id"], 42);
        assert_eq!(json["persistenceData"]["saved"], true);
    }

    #[test]
    fn deserialize_next_result() {
        let json = serde_json::json!({"proceed": true});
        let result = deserialize_execution_result(&json).unwrap();
        assert!(result.proceed);
        assert!(result.outcome_value.is_none());
    }

    #[test]
    fn deserialize_outcome_result() {
        let json = serde_json::json!({
            "proceed": true,
            "outcomeValue": "branch-a"
        });
        let result = deserialize_execution_result(&json).unwrap();
        assert!(result.proceed);
        assert_eq!(result.outcome_value, Some(serde_json::json!("branch-a")));
    }

    #[test]
    fn deserialize_sleep_result() {
        let json = serde_json::json!({
            "proceed": false,
            "sleepFor": 5000
        });
        let result = deserialize_execution_result(&json).unwrap();
        assert!(!result.proceed);
        assert_eq!(result.sleep_for, Some(Duration::from_millis(5000)));
    }

    #[test]
    fn deserialize_persist_result() {
        let json = serde_json::json!({
            "proceed": false,
            "persistenceData": {"page": 2}
        });
        let result = deserialize_execution_result(&json).unwrap();
        assert!(!result.proceed);
        assert_eq!(
            result.persistence_data,
            Some(serde_json::json!({"page": 2}))
        );
    }

    #[test]
    fn deserialize_wait_for_event_result() {
        let json = serde_json::json!({
            "proceed": false,
            "eventName": "order.paid",
            "eventKey": "order-123"
        });
        let result = deserialize_execution_result(&json).unwrap();
        assert_eq!(result.event_name, Some("order.paid".into()));
        assert_eq!(result.event_key, Some("order-123".into()));
    }

    #[test]
    fn deserialize_branch_result() {
        let json = serde_json::json!({
            "proceed": false,
            "branchValues": [1, 2, 3]
        });
        let result = deserialize_execution_result(&json).unwrap();
        assert_eq!(
            result.branch_values,
            Some(vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3)
            ])
        );
    }

    #[test]
    fn deserialize_output_data_result() {
        let json = serde_json::json!({
            "proceed": true,
            "outputData": {"greeted": "World"}
        });
        let result = deserialize_execution_result(&json).unwrap();
        assert!(result.proceed);
        assert_eq!(
            result.output_data,
            Some(serde_json::json!({"greeted": "World"}))
        );
    }

    #[test]
    fn deserialize_empty_object_defaults_to_proceed() {
        let json = serde_json::json!({});
        let result = deserialize_execution_result(&json).unwrap();
        assert!(result.proceed);
    }

    #[test]
    fn deserialize_invalid_json_returns_error() {
        let json = serde_json::json!("not an object");
        let result = deserialize_execution_result(&json);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn js_step_body_channel_round_trip() {
        let (tx, mut rx) = mpsc::channel::<StepRequest>(16);
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut body = JsStepBody::new(tx, counter);

        let (instance, step, pointer) = make_test_context();
        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
        };

        // Spawn a "JS side" that responds to the request.
        let responder = tokio::spawn(async move {
            let req = rx.recv().await.unwrap();
            assert_eq!(req.step_type, "MyStep");
            assert_eq!(req.request_id, 0);
            req.response_tx
                .send(Ok(serde_json::json!({"proceed": true, "outputData": {"done": true}})))
                .unwrap();
        });

        let result = body.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert_eq!(result.output_data, Some(serde_json::json!({"done": true})));

        responder.await.unwrap();
    }

    #[tokio::test]
    async fn js_step_body_propagates_js_error() {
        let (tx, mut rx) = mpsc::channel::<StepRequest>(16);
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut body = JsStepBody::new(tx, counter);

        let (instance, step, pointer) = make_test_context();
        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
        };

        tokio::spawn(async move {
            let req = rx.recv().await.unwrap();
            req.response_tx
                .send(Err("TypeError: undefined is not a function".into()))
                .unwrap();
        });

        let result = body.run(&ctx).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("TypeError"));
    }

    #[tokio::test]
    async fn js_step_body_handles_dropped_responder() {
        let (tx, mut rx) = mpsc::channel::<StepRequest>(16);
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut body = JsStepBody::new(tx, counter);

        let (instance, step, pointer) = make_test_context();
        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: None,
            log_sink: None,
        };

        tokio::spawn(async move {
            let req = rx.recv().await.unwrap();
            drop(req.response_tx); // Drop without sending
        });

        let result = body.run(&ctx).await;
        assert!(result.is_err());
    }
}
