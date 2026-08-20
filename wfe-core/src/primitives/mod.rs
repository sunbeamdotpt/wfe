//! Built-in control-flow primitives for workflow definitions.
//!
//! These steps implement [`StepBody`](crate::traits::step::StepBody) and can be
//! composed via the builder API to create loops, conditionals, parallel branches,
//! and more without writing custom code.
//!
//! | Primitive | Builder method | Description |
//! |-----------|---------------|-------------|
//! | [`DecideStep`](crate::primitives::DecideStep) | — (used by `if_do`) | Return an outcome value for routing |
//! | [`DelayStep`](crate::primitives::DelayStep) | `StepBuilder::delay` | Sleep for a duration |
//! | [`EndStep`](crate::primitives::EndStep) | `StepBuilder::end_workflow` | Terminate the workflow |
//! | [`ForEachStep`](crate::primitives::ForEachStep) | `StepBuilder::for_each` | Iterate over a collection |
//! | [`IfStep`](crate::primitives::IfStep) | `StepBuilder::if_do` | Conditional branch |
//! | [`PollEndpointStep`](crate::primitives::PollEndpointStep) | — | Poll an HTTP endpoint until success |
//! | [`RecurStep`](crate::primitives::RecurStep) | — | Schedule a recurring workflow |
//! | [`SagaContainerStep`](crate::primitives::SagaContainerStep) | `StepBuilder::saga` | Saga with compensation steps |
//! | [`ScheduleStep`](crate::primitives::ScheduleStep) | — | Schedule a command for later execution |
//! | [`SequenceStep`](crate::primitives::SequenceStep) | — | Run a child workflow as a step |
//! | [`SubWorkflowStep`](crate::primitives::SubWorkflowStep) | `StepBuilder::add_child` | Embed a child workflow |
//! | [`WaitForStep`](crate::primitives::WaitForStep) | `StepBuilder::wait_for` | Block until an event arrives |
//! | [`WhileStep`](crate::primitives::WhileStep) | `StepBuilder::while_do` | Loop while a condition holds |

/// Multi-way branch primitive.
pub mod decide;
/// Delay/sleep primitive.
pub mod delay;
/// Workflow termination primitive.
pub mod end_step;
/// Collection iteration primitive.
pub mod foreach_step;
/// Conditional branch primitive.
pub mod if_step;
/// HTTP polling primitive.
pub mod poll_endpoint;
/// Recurring execution primitive.
pub mod recur;
/// Saga compensation container primitive.
pub mod saga_container;
/// Scheduled command primitive.
pub mod schedule;
/// Parallel sequence container primitive.
pub mod sequence;
/// Child workflow primitive.
pub mod sub_workflow;
/// Event wait primitive.
pub mod wait_for;
/// While-loop primitive.
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
            artifact_store: None,
            artifact_volume: None,
            artifact_package: None,
            persistence: None,
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
