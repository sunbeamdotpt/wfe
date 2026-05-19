use async_trait::async_trait;
use chrono::Utc;

use crate::models::ExecutionResult;
use crate::models::schema::WorkflowSchema;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that starts a child workflow and waits for its completion.
///
/// On first invocation, it validates inputs against `input_schema`, starts the
/// child workflow via the host context, and returns a "wait for event" result.
///
/// When the child workflow completes, the event data arrives, output keys are
/// extracted, and the step proceeds.
#[derive(Default)]
pub struct SubWorkflowStep {
    /// The definition ID of the child workflow to start.
    pub workflow_id: String,
    /// The version of the child workflow definition.
    pub version: u32,
    /// Input data to pass to the child workflow.
    pub inputs: serde_json::Value,
    /// Keys to extract from the child workflow's completion event data.
    pub output_keys: Vec<String>,
    /// Optional schema to validate inputs before starting the child.
    pub input_schema: Option<WorkflowSchema>,
    /// Optional schema to validate outputs from the child.
    pub output_schema: Option<WorkflowSchema>,
}

#[async_trait]
impl StepBody for SubWorkflowStep {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        // If event data has arrived, the child workflow completed.
        if let Some(event_data) = &context.execution_pointer.event_data {
            // Extract output_keys from event data.
            let mut output = serde_json::Map::new();

            // The event data contains { "status": "...", "data": { ... } }.
            let child_data = event_data
                .get("data")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            if self.output_keys.is_empty() {
                // If no specific keys requested, pass all child data through.
                if let serde_json::Value::Object(map) = child_data {
                    output = map;
                }
            } else {
                // Extract only the requested keys.
                for key in &self.output_keys {
                    if let Some(val) = child_data.get(key) {
                        output.insert(key.clone(), val.clone());
                    }
                }
            }

            let output_value = serde_json::Value::Object(output);

            // Validate against output schema if present.
            if let Some(ref schema) = self.output_schema
                && let Err(errors) = schema.validate_outputs(&output_value)
            {
                return Err(crate::WfeError::StepExecution(format!(
                    "SubWorkflow output validation failed: {}",
                    errors.join("; ")
                )));
            }

            let mut result = ExecutionResult::next();
            result.output_data = Some(output_value);
            return Ok(result);
        }

        // Hydrate from step_config if our fields are empty (created via Default).
        if self.workflow_id.is_empty()
            && let Some(config) = &context.step.step_config
        {
            if let Some(wf_id) = config.get("workflow_id").and_then(|v| v.as_str()) {
                self.workflow_id = wf_id.to_string();
            }
            if let Some(ver) = config.get("version").and_then(|v| v.as_u64()) {
                self.version = ver as u32;
            }
            if let Some(inputs) = config.get("inputs") {
                self.inputs = inputs.clone();
            }
            if let Some(keys) = config.get("output_keys").and_then(|v| v.as_array()) {
                self.output_keys = keys
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
            }
        }

        // First call: validate inputs and start child workflow.
        if let Some(ref schema) = self.input_schema
            && let Err(errors) = schema.validate_inputs(&self.inputs)
        {
            return Err(crate::WfeError::StepExecution(format!(
                "SubWorkflow input validation failed: {}",
                errors.join("; ")
            )));
        }

        let host = context.host_context.ok_or_else(|| {
            crate::WfeError::StepExecution(
                "SubWorkflowStep requires a host context to start child workflows".to_string(),
            )
        })?;

        // Use explicit inputs if set; otherwise inherit the parent workflow's
        // data so child steps can reference the same top-level fields (e.g.
        // REPO_URL, COMMIT_SHA) without every `type: workflow` step having to
        // re-declare them. Fall back to an empty object when the parent has
        // no data either so the child still has a valid JSON object for
        // storing step outputs.
        let child_data = if !self.inputs.is_null() {
            self.inputs.clone()
        } else if context.workflow.data.is_object() {
            context.workflow.data.clone()
        } else {
            serde_json::json!({})
        };
        // Inherit the parent's root — or, if the parent is itself a root
        // (has no root set), use the parent's own id as the root for the
        // child. This makes every descendant of a top-level ci run share
        // the same root_workflow_id and therefore the same namespace and
        // shared volume on backends that care.
        let parent_root = context
            .workflow
            .root_workflow_id
            .clone()
            .or_else(|| Some(context.workflow.id.clone()));
        let child_instance_id = host
            .start_workflow(&self.workflow_id, self.version, child_data, parent_root)
            .await?;

        Ok(ExecutionResult::wait_for_event(
            "wfe.workflow.completed",
            child_instance_id,
            Utc::now(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionPointer;
    use crate::models::schema::SchemaType;
    use crate::primitives::test_helpers::*;
    use crate::traits::step::HostContext;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A mock HostContext that records calls and returns a fixed instance ID.
    struct MockHostContext {
        started: Mutex<Vec<(String, u32, serde_json::Value)>>,
        result_id: String,
    }

    impl MockHostContext {
        fn new(result_id: &str) -> Self {
            Self {
                started: Mutex::new(Vec::new()),
                result_id: result_id.to_string(),
            }
        }

        fn calls(&self) -> Vec<(String, u32, serde_json::Value)> {
            self.started.lock().unwrap().clone()
        }
    }

    impl HostContext for MockHostContext {
        fn start_workflow(
            &self,
            definition_id: &str,
            version: u32,
            data: serde_json::Value,
            _parent_root_workflow_id: Option<String>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<String>> + Send + '_>>
        {
            let def_id = definition_id.to_string();
            let result_id = self.result_id.clone();
            Box::pin(async move {
                self.started.lock().unwrap().push((def_id, version, data));
                Ok(result_id)
            })
        }
    }

    /// A mock HostContext that returns an error.
    struct FailingHostContext;

    impl HostContext for FailingHostContext {
        fn start_workflow(
            &self,
            _definition_id: &str,
            _version: u32,
            _data: serde_json::Value,
            _parent_root_workflow_id: Option<String>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<String>> + Send + '_>>
        {
            Box::pin(async {
                Err(crate::WfeError::StepExecution(
                    "failed to start child".to_string(),
                ))
            })
        }
    }

    fn make_context_with_host<'a>(
        pointer: &'a ExecutionPointer,
        step: &'a crate::models::WorkflowStep,
        workflow: &'a crate::models::WorkflowInstance,
        host: &'a dyn HostContext,
    ) -> StepExecutionContext<'a> {
        StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: pointer,
            persistence_data: pointer.persistence_data.as_ref(),
            step,
            workflow,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            host_context: Some(host),
            log_sink: None,
            artifact_store: None,
        }
    }

    #[tokio::test]
    async fn first_call_starts_child_and_waits() {
        let host = MockHostContext::new("child-123");
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({"x": 10}),
            ..Default::default()
        };

        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context_with_host(&pointer, &wf_step, &workflow, &host);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.event_name.as_deref(), Some("wfe.workflow.completed"));
        assert_eq!(result.event_key.as_deref(), Some("child-123"));
        assert!(result.event_as_of.is_some());

        let calls = host.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "child-def");
        assert_eq!(calls[0].1, 1);
        assert_eq!(calls[0].2, json!({"x": 10}));
    }

    #[tokio::test]
    async fn child_completed_proceeds_with_output() {
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({}),
            output_keys: vec!["result".into()],
            ..Default::default()
        };

        let mut pointer = ExecutionPointer::new(0);
        pointer.event_data = Some(json!({
            "status": "Complete",
            "data": {"result": "success", "extra": "ignored"}
        }));
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert_eq!(result.output_data, Some(json!({"result": "success"})));
    }

    #[tokio::test]
    async fn child_completed_no_output_keys_passes_all() {
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({}),
            output_keys: vec![],
            ..Default::default()
        };

        let mut pointer = ExecutionPointer::new(0);
        pointer.event_data = Some(json!({
            "status": "Complete",
            "data": {"a": 1, "b": 2}
        }));
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert_eq!(result.output_data, Some(json!({"a": 1, "b": 2})));
    }

    #[tokio::test]
    async fn no_host_context_errors() {
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({}),
            ..Default::default()
        };

        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let err = step.run(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("host context"));
    }

    #[tokio::test]
    async fn input_validation_failure() {
        let host = MockHostContext::new("child-123");
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({"name": 42}), // wrong type
            input_schema: Some(WorkflowSchema {
                inputs: HashMap::from([("name".into(), SchemaType::String)]),
                outputs: HashMap::new(),
            }),
            ..Default::default()
        };

        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context_with_host(&pointer, &wf_step, &workflow, &host);

        let err = step.run(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("input validation failed"));
        assert!(host.calls().is_empty());
    }

    #[tokio::test]
    async fn output_validation_failure() {
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({}),
            output_keys: vec![],
            output_schema: Some(WorkflowSchema {
                inputs: HashMap::new(),
                outputs: HashMap::from([("result".into(), SchemaType::String)]),
            }),
            ..Default::default()
        };

        let mut pointer = ExecutionPointer::new(0);
        pointer.event_data = Some(json!({
            "status": "Complete",
            "data": {"result": 42}
        }));
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let err = step.run(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("output validation failed"));
    }

    #[tokio::test]
    async fn input_validation_passes_then_starts_child() {
        let host = MockHostContext::new("child-456");
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 2,
            inputs: json!({"name": "Alice"}),
            input_schema: Some(WorkflowSchema {
                inputs: HashMap::from([("name".into(), SchemaType::String)]),
                outputs: HashMap::new(),
            }),
            ..Default::default()
        };

        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context_with_host(&pointer, &wf_step, &workflow, &host);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.event_key.as_deref(), Some("child-456"));
        assert_eq!(host.calls().len(), 1);
    }

    #[tokio::test]
    async fn host_start_workflow_error_propagates() {
        let host = FailingHostContext;
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({}),
            ..Default::default()
        };

        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context_with_host(&pointer, &wf_step, &workflow, &host);

        let err = step.run(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("failed to start child"));
    }

    #[tokio::test]
    async fn event_data_without_data_field_returns_empty_output() {
        let mut step = SubWorkflowStep {
            workflow_id: "child-def".into(),
            version: 1,
            inputs: json!({}),
            output_keys: vec!["foo".into()],
            ..Default::default()
        };

        let mut pointer = ExecutionPointer::new(0);
        pointer.event_data = Some(json!({"status": "Complete"}));
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(result.proceed);
        assert_eq!(result.output_data, Some(json!({})));
    }

    #[tokio::test]
    async fn default_step_has_empty_fields() {
        let step = SubWorkflowStep::default();
        assert!(step.workflow_id.is_empty());
        assert_eq!(step.version, 0);
        assert_eq!(step.inputs, json!(null));
        assert!(step.output_keys.is_empty());
        assert!(step.input_schema.is_none());
        assert!(step.output_schema.is_none());
    }
}
