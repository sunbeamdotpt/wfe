/// Decide.
pub mod decide;
/// Delay.
pub mod delay;
/// End step.
pub mod end_step;
/// Foreach step.
pub mod foreach_step;
/// If step.
pub mod if_step;
/// Poll endpoint.
pub mod poll_endpoint;
/// Recur.
pub mod recur;
/// Saga container.
pub mod saga_container;
/// Schedule.
pub mod schedule;
/// Sequence.
pub mod sequence;
/// Sub workflow.
pub mod sub_workflow;
/// Wait for.
pub mod wait_for;
/// While step.
pub mod while_step;

pub use decide::DecideStep;
pub use delay::DelayStep;
pub use end_step::EndStep;
pub use foreach_step::ForEachStep;
pub use if_step::IfStep;
pub use poll_endpoint::PollEndpointStep;
pub use recur::RecurStep;
pub use saga_container::SagaContainerStep;
pub use schedule::ScheduleStep;
pub use sequence::SequenceStep;
pub use sub_workflow::SubWorkflowStep;
pub use wait_for::WaitForStep;
pub use while_step::WhileStep;

#[cfg(test)]
mod test_helpers {
    use crate::models::{ExecutionPointer, WorkflowInstance, WorkflowStep};
    use crate::traits::step::StepExecutionContext;
    use tokio_util::sync::CancellationToken;

/// Make context.
    pub fn make_context<'a>(
        pointer: &'a ExecutionPointer,
        step: &'a WorkflowStep,
        workflow: &'a WorkflowInstance,
    ) -> StepExecutionContext<'a> {
        StepExecutionContext {
            definition: None,
            item: None,
            execution_pointer: pointer,
            persistence_data: pointer.persistence_data.as_ref(),
            step,
            workflow,
            cancellation_token: CancellationToken::new(),
            host_context: None,
            log_sink: None,
        }
    }

/// Default workflow.
    pub fn default_workflow() -> WorkflowInstance {
        WorkflowInstance::new("test-workflow", 1, serde_json::json!({}))
    }

/// Default step.
    pub fn default_step() -> WorkflowStep {
        WorkflowStep::new(0, "TestStep")
    }
}
