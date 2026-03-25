use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::models::{ExecutionPointer, ExecutionResult, WorkflowInstance, WorkflowStep};

/// Marker trait for all data types that flow between workflow steps.
/// Anything that is serializable and deserializable qualifies.
pub trait WorkflowData: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {}

/// Blanket implementation: any type satisfying the bounds is WorkflowData.
impl<T> WorkflowData for T where T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {}

/// Context available to a step during execution.
#[derive(Debug)]
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
}

/// The core unit of work in a workflow. Each step implements this trait.
#[async_trait]
pub trait StepBody: Send + Sync {
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult>;
}
