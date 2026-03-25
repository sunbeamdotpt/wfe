use crate::models::{ErrorBehavior, ExecutionResult};
use crate::primitives;
use crate::traits::step::{StepBody, WorkflowData};

use super::inline_step::InlineStep;
use super::workflow_builder::WorkflowBuilder;

/// Builder for configuring a single step in the workflow.
///
/// Owns the WorkflowBuilder, consuming self on each method call.
/// This avoids all lifetime/borrow issues.
pub struct StepBuilder<D: WorkflowData> {
    builder: WorkflowBuilder<D>,
    step_id: usize,
}

/// Builder for parallel branches.
pub struct ParallelBuilder<D: WorkflowData> {
    builder: WorkflowBuilder<D>,
    container_id: usize,
}

impl<D: WorkflowData> StepBuilder<D> {
    pub(crate) fn new(builder: WorkflowBuilder<D>, step_id: usize) -> Self {
        Self { builder, step_id }
    }

    /// Set the display name of the current step.
    pub fn name(mut self, name: &str) -> Self {
        self.builder.steps[self.step_id].name = Some(name.to_string());
        self
    }

    /// Set an external ID for forward references.
    pub fn id(mut self, external_id: &str) -> Self {
        self.builder.steps[self.step_id].external_id = Some(external_id.to_string());
        self
    }

    /// Set the error handling behavior for this step.
    pub fn on_error(mut self, behavior: ErrorBehavior) -> Self {
        self.builder.steps[self.step_id].error_behavior = Some(behavior);
        self
    }

    /// Add a compensation step for saga rollback.
    pub fn compensate_with<C: StepBody + Default + 'static>(mut self) -> Self {
        let comp_id = self.builder.add_step(std::any::type_name::<C>());
        self.builder.steps[self.step_id].compensation_step_id = Some(comp_id);
        self
    }

    /// Chain the next step. Wires an outcome from the current step to the new one.
    pub fn then<S: StepBody + Default + 'static>(mut self) -> StepBuilder<D> {
        let next_id = self.builder.add_step(std::any::type_name::<S>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        StepBuilder::new(self.builder, next_id)
    }

    /// Chain an inline function step.
    pub fn then_fn(mut self, f: impl Fn() -> ExecutionResult + Send + Sync + 'static) -> StepBuilder<D> {
        let next_id = self.builder.add_step(std::any::type_name::<InlineStep>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        self.builder.inline_closures.insert(next_id, Box::new(f));
        StepBuilder::new(self.builder, next_id)
    }

    /// Insert a WaitFor step.
    pub fn wait_for(mut self, event_name: &str, event_key: &str) -> StepBuilder<D> {
        let next_id = self.builder.add_step(std::any::type_name::<primitives::wait_for::WaitForStep>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        self.builder.steps[next_id].step_config = Some(serde_json::json!({
            "event_name": event_name,
            "event_key": event_key,
        }));
        StepBuilder::new(self.builder, next_id)
    }

    /// Insert a Delay step.
    pub fn delay(mut self, duration: std::time::Duration) -> StepBuilder<D> {
        let next_id = self.builder.add_step(std::any::type_name::<primitives::delay::DelayStep>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        self.builder.steps[next_id].step_config = Some(serde_json::json!({
            "duration_millis": duration.as_millis() as u64,
        }));
        StepBuilder::new(self.builder, next_id)
    }

    /// Insert an If container step with child steps built by the closure.
    /// The type parameter S is the step type used for the If condition evaluation.
    pub fn if_do<S: StepBody + Default + 'static>(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let if_id = self.builder.add_step(std::any::type_name::<primitives::if_step::IfStep>());
        self.builder.wire_outcome(self.step_id, if_id, None);

        // Build children
        let before_count = self.builder.steps.len();
        build_children(&mut self.builder);
        let after_count = self.builder.steps.len();

        // Register children with the If step
        for child_id in before_count..after_count {
            self.builder.add_child(if_id, child_id);
        }

        self.builder.last_step = Some(if_id);
        StepBuilder::new(self.builder, if_id)
    }

    /// Insert a While container step.
    pub fn while_do<S: StepBody + Default + 'static>(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let while_id = self.builder.add_step(std::any::type_name::<primitives::while_step::WhileStep>());
        self.builder.wire_outcome(self.step_id, while_id, None);

        let before_count = self.builder.steps.len();
        build_children(&mut self.builder);
        let after_count = self.builder.steps.len();

        for child_id in before_count..after_count {
            self.builder.add_child(while_id, child_id);
        }

        self.builder.last_step = Some(while_id);
        StepBuilder::new(self.builder, while_id)
    }

    /// Insert a ForEach container step.
    pub fn for_each<S: StepBody + Default + 'static>(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let fe_id = self.builder.add_step(std::any::type_name::<primitives::foreach_step::ForEachStep>());
        self.builder.wire_outcome(self.step_id, fe_id, None);

        let before_count = self.builder.steps.len();
        build_children(&mut self.builder);
        let after_count = self.builder.steps.len();

        for child_id in before_count..after_count {
            self.builder.add_child(fe_id, child_id);
        }

        self.builder.last_step = Some(fe_id);
        StepBuilder::new(self.builder, fe_id)
    }

    /// Insert a Saga container step with child steps.
    pub fn saga(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let saga_id = self.builder.add_step(std::any::type_name::<primitives::saga_container::SagaContainerStep>());
        self.builder.steps[saga_id].saga = true;
        self.builder.wire_outcome(self.step_id, saga_id, None);

        let before_count = self.builder.steps.len();
        build_children(&mut self.builder);
        let after_count = self.builder.steps.len();

        for child_id in before_count..after_count {
            self.builder.add_child(saga_id, child_id);
        }

        self.builder.last_step = Some(saga_id);
        StepBuilder::new(self.builder, saga_id)
    }

    /// Start a parallel block.
    pub fn parallel(
        mut self,
        build_branches: impl FnOnce(ParallelBuilder<D>) -> ParallelBuilder<D>,
    ) -> StepBuilder<D> {
        let seq_id = self.builder.add_step(std::any::type_name::<primitives::sequence::SequenceStep>());
        self.builder.wire_outcome(self.step_id, seq_id, None);

        let pb = ParallelBuilder {
            builder: self.builder,
            container_id: seq_id,
        };
        let pb = build_branches(pb);
        let mut builder = pb.builder;
        builder.last_step = Some(seq_id);
        StepBuilder::new(builder, seq_id)
    }

    /// Mark this step as the terminal step (no further outcomes).
    pub fn end_workflow(self) -> WorkflowBuilder<D> {
        self.builder
    }

    /// Access the compiled definition directly (shortcut for end_workflow().build()).
    pub fn build(self, id: impl Into<String>, version: u32) -> crate::models::WorkflowDefinition {
        self.builder.build(id, version)
    }
}

impl<D: WorkflowData> ParallelBuilder<D> {
    /// Add a parallel branch.
    pub fn branch(
        mut self,
        build_branch: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> Self {
        let before_count = self.builder.steps.len();
        build_branch(&mut self.builder);
        let after_count = self.builder.steps.len();

        for child_id in before_count..after_count {
            self.builder.add_child(self.container_id, child_id);
        }
        self
    }
}
