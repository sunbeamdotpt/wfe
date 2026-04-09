pub mod decide;
pub mod delay;
pub mod end_step;
pub mod foreach_step;
pub mod if_step;
pub mod poll_endpoint;
pub mod recur;
pub mod saga_container;
pub mod schedule;
pub mod sequence;
pub mod sub_workflow;
pub mod wait_for;
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

    pub fn default_workflow() -> WorkflowInstance {
        WorkflowInstance::new("test-workflow", 1, serde_json::json!({}))
    }

    pub fn default_step() -> WorkflowStep {
        WorkflowStep::new(0, "TestStep")
    }
}
