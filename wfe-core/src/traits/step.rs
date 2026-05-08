use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::models::{
    ExecutionPointer, ExecutionResult, WorkflowDefinition, WorkflowInstance, WorkflowStep,
};

/// Marker trait for all data types that flow between workflow steps.
/// Anything that is serializable and deserializable qualifies.
pub trait WorkflowData: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {}

/// Blanket implementation: any type satisfying the bounds is WorkflowData.
impl<T> WorkflowData for T where T: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {}

/// Context for steps that need to interact with the workflow host.
/// Implemented by WorkflowHost to allow steps like SubWorkflow to start child workflows.
///
/// The `parent_root_workflow_id` argument carries the UUID of the top-level
/// ancestor workflow so backends (notably Kubernetes) can place every
/// descendant of a given root run in the same isolation domain — namespace,
/// shared volume, RBAC — so sub-workflows can share state like a cloned
/// repo checkout. Pass `None` when starting a brand-new root workflow.
pub trait HostContext: Send + Sync {
    fn start_workflow(
        &self,
        definition_id: &str,
        version: u32,
        data: serde_json::Value,
        parent_root_workflow_id: Option<String>,
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
    /// The compiled workflow definition the instance was created from.
    /// `None` on code paths that don't have it available (some test fixtures);
    /// production execution always populates this so executor-specific
    /// features (e.g. Kubernetes shared volumes) can inspect the
    /// definition-level configuration.
    pub definition: Option<&'a WorkflowDefinition>,
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
            .field("definition", &self.definition.is_some())
            .field("host_context", &self.host_context.is_some())
            .field("log_sink", &self.log_sink.is_some())
            .finish()
    }
}

/// The core unit of work in a workflow. Each step implements this trait.
///
/// Steps must be `Send + Sync` because the executor may run them on different
/// threads. They must also implement `Default` to be usable with the builder API.
///
/// # Example
/// ```ignore
/// use async_trait::async_trait;
/// use wfe_core::models::ExecutionResult;
/// use wfe_core::traits::step::{StepBody, StepExecutionContext};
///
/// #[derive(Default)]
/// struct Greet;
///
/// #[async_trait]
/// impl StepBody for Greet {
///     async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
///         println!("Hello, workflow!");
///         Ok(ExecutionResult::next())
///     }
/// }
/// ```
#[async_trait]
pub trait StepBody: Send + Sync {
    /// Execute the step.
    ///
    /// The [`StepExecutionContext`] provides access to workflow data, the current
    /// execution pointer, persistence data from previous runs, and a cancellation
    /// token. Return an [`ExecutionResult`] to control flow:
    ///
    /// - [`ExecutionResult::next()`](crate::models::ExecutionResult::next) — continue to the next step
    /// - [`ExecutionResult::branch(values, data)`](crate::models::ExecutionResult::branch) — follow a named outcome
    /// - [`ExecutionResult::sleep(duration, data)`](crate::models::ExecutionResult::sleep) — pause execution
    /// - `Err(WfeError::Execution("msg".into()))` — mark the pointer as failed
    /// - [`ExecutionResult::persist(data)`](crate::models::ExecutionResult::persist) — persist state and pause
    async fn run(&mut self, context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult>;
}
