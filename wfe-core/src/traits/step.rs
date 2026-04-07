use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::models::{ExecutionPointer, ExecutionResult, WorkflowInstance, WorkflowStep};

/// Marker trait for all data types that flow between workflow steps.
/// Anything that is serializable and deserializable qualifies.
pub trait WorkflowData: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {}

/// Blanket implementation: any type satisfying the bounds is WorkflowData.
impl<T> WorkflowData for T where T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {}

/// Context for steps that need to interact with the workflow host.
/// Implemented by WorkflowHost to allow steps like SubWorkflow to start child workflows.
pub trait HostContext: Send + Sync {
    fn start_workflow(
        &self,
        definition_id: &str,
        version: u32,
        data: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = crate::Result<String>> + Send + '_>>;
}

/// Context available to a step during execution.
pub struct StepExecutionContext<'a> {
    /// The current item when iterating (ForEach).
    pub item: Option<&'a serde_json::Value>,
    /// The current execution pointer.
    pub execution_pointer: &'a ExecutionPointer,
    /// Persistence data from a previous execution of this step.
    pub persistence_data: Option<&'a serde_json::Value>,
    /// The step definition.
    pub step: &'a WorkflowStep,
    /// The running workflow instance.
    pub workflow: &'a WorkflowInstance,
    /// Cancellation token.
    pub cancellation_token: tokio_util::sync::CancellationToken,
    /// Host context for starting child workflows. None if not available.
    pub host_context: Option<&'a dyn HostContext>,
    /// Log sink for streaming step output. None if not configured.
    pub log_sink: Option<&'a dyn super::LogSink>,
}

// Manual Debug impl since dyn HostContext is not Debug.
impl<'a> std::fmt::Debug for StepExecutionContext<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StepExecutionContext")
            .field("item", &self.item)
            .field("execution_pointer", &self.execution_pointer)
            .field("persistence_data", &self.persistence_data)
            .field("step", &self.step)
            .field("workflow", &self.workflow)
            .field("host_context", &self.host_context.is_some())
            .field("log_sink", &self.log_sink.is_some())
            .finish()
    }
}

/// The core unit of work in a workflow. Each step implements this trait.
#[async_trait]
pub trait StepBody: Send + Sync {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult>;
}
