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

    /// Set the human-readable display name of the current step.
    ///
    /// This name appears in logs, the execution trace, and the web UI.
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
    ///
    /// When a step returns `Err` from its `run` method,
    /// the executor checks this behavior to decide what to do next.
    ///
    /// # Example
    /// ```ignore
    /// .then::< risky::Step>()
    ///     .name("Risky")
    ///     .on_error(ErrorBehavior::Retry {
    ///         interval: Duration::from_secs(5),
    ///         max_retries: 3,
    ///     })
    /// ```
    pub fn on_error(mut self, behavior: ErrorBehavior) -> Self {
        self.builder.steps[self.step_id].error_behavior = Some(behavior);
        self
    }

    /// Attach arbitrary JSON configuration to this step.
    ///
    /// The step can read it at runtime via `context.step.step_config`.
    pub fn config(mut self, config: serde_json::Value) -> Self {
        self.builder.steps[self.step_id].step_config = Some(config);
        self
    }

    /// Register a compensation step for saga rollback.
    ///
    /// When this step fails inside a [`saga`](Self::saga) container, the executor
    /// runs the compensation steps in reverse order to undo partial work.
    ///
    /// # Example
    /// ```ignore
    /// .then::<ChargeCard>()
    ///     .name("Charge")
    ///     .compensate_with::<RefundCard>()
    /// ```
    pub fn compensate_with<C: StepBody + Default + 'static>(mut self) -> Self {
        let comp_id = self.builder.add_step(std::any::type_name::<C>());
        self.builder.steps[self.step_id].compensation_step_id = Some(comp_id);
        self
    }

    /// Chain the next step sequentially.
    ///
    /// Wires an outcome from the current step to the new one so that when the
    /// current step returns [`ExecutionResult::next`](crate::models::ExecutionResult::next),
    /// execution continues with `S`.
    pub fn then<S: StepBody + Default + 'static>(mut self) -> StepBuilder<D> {
        let next_id = self.builder.add_step(std::any::type_name::<S>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        StepBuilder::new(self.builder, next_id)
    }

    /// Chain an inline function step.
    pub fn then_fn(
        mut self,
        f: impl Fn() -> ExecutionResult + Send + Sync + 'static,
    ) -> StepBuilder<D> {
        let next_id = self.builder.add_step(std::any::type_name::<InlineStep>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        self.builder.inline_closures.insert(next_id, Box::new(f));
        StepBuilder::new(self.builder, next_id)
    }

    /// Suspend the workflow until an external event arrives.
    ///
    /// The workflow pauses and the executor releases the lock. When you call
    /// `WorkflowHost::publish_event` (from the `wfe` crate) with
    /// a matching `event_name` and `event_key`, the workflow resumes from this point.
    ///
    /// # Example
    /// ```ignore
    /// .then::<RequestApproval>()
    ///     .name("Request Approval")
    /// .wait_for("approval", "order-123")
    ///     .name("Wait for approval")
    /// ```
    pub fn wait_for(mut self, event_name: &str, event_key: &str) -> StepBuilder<D> {
        let next_id = self
            .builder
            .add_step(std::any::type_name::<primitives::wait_for::WaitForStep>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        self.builder.steps[next_id].step_config = Some(serde_json::json!({
            "event_name": event_name,
            "event_key": event_key,
        }));
        StepBuilder::new(self.builder, next_id)
    }

    /// Pause execution for a fixed duration.
    ///
    /// The executor persists the workflow, sleeps for the given duration, then
    /// re-queues the instance for continued execution.
    pub fn delay(mut self, duration: std::time::Duration) -> StepBuilder<D> {
        let next_id = self
            .builder
            .add_step(std::any::type_name::<primitives::delay::DelayStep>());
        self.builder.wire_outcome(self.step_id, next_id, None);
        self.builder.last_step = Some(next_id);
        self.builder.steps[next_id].step_config = Some(serde_json::json!({
            "duration_millis": duration.as_millis() as u64,
        }));
        StepBuilder::new(self.builder, next_id)
    }

    /// Conditional branching.
    ///
    /// The closure builds the child steps that run when the condition evaluates
    /// to `true`. Use `.then::<ConditionStep>().if_do(|b| { ... })` where
    /// `ConditionStep` returns [`ExecutionResult::branch("true")`](crate::models::ExecutionResult::branch)
    /// or `branch("false")` to control which path is taken.
    pub fn if_do<S: StepBody + Default + 'static>(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let if_id = self
            .builder
            .add_step(std::any::type_name::<primitives::if_step::IfStep>());
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

    /// Loop while a condition holds.
    ///
    /// The closure builds the body of the loop. A condition step (type `S`)
    /// should return [`ExecutionResult::next()`](crate::models::ExecutionResult::next)
    /// to continue looping or `ExecutionResult::next()` with a condition that evaluates to `false`
    /// to break out.
    pub fn while_do<S: StepBody + Default + 'static>(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let while_id = self
            .builder
            .add_step(std::any::type_name::<primitives::while_step::WhileStep>());
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

    /// Iterate over a collection.
    ///
    /// The step type `S` receives each item via
    /// [`StepExecutionContext::item`](crate::traits::step::StepExecutionContext::item).
    /// The collection is taken from the workflow data field configured in the
    /// step's `step_config`.
    pub fn for_each<S: StepBody + Default + 'static>(
        mut self,
        build_children: impl FnOnce(&mut WorkflowBuilder<D>),
    ) -> StepBuilder<D> {
        let fe_id = self
            .builder
            .add_step(std::any::type_name::<primitives::foreach_step::ForEachStep>());
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

    /// Transaction-like container with compensation on failure.
    ///
    /// Child steps run normally. If any child fails, the executor runs
    /// compensation steps (registered via [`compensate_with`](Self::compensate_with))
    /// in reverse order to undo partial work.
    ///
    /// # Example
    /// ```ignore
    /// .saga(|b| {
    ///     b.add_step_typed::<ReserveInventory>("reserve", None);
    ///     b.add_step_typed::<ChargeCard>("charge", None);
    ///     b.add_step_typed::<ShipOrder>("ship", None);
    /// })
    /// ```
    pub fn saga(mut self, build_children: impl FnOnce(&mut WorkflowBuilder<D>)) -> StepBuilder<D> {
        let saga_id = self.builder.add_step(std::any::type_name::<
            primitives::saga_container::SagaContainerStep,
        >());
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

    /// Run multiple branches concurrently.
    ///
    /// All branches inside the [`ParallelBuilder`] execute in parallel. The
    /// workflow continues to the next step only after **all** branches complete.
    pub fn parallel(
        mut self,
        build_branches: impl FnOnce(ParallelBuilder<D>) -> ParallelBuilder<D>,
    ) -> StepBuilder<D> {
        let seq_id = self
            .builder
            .add_step(std::any::type_name::<primitives::sequence::SequenceStep>());
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

    /// Finish building the current branch and return the [`WorkflowBuilder`].
    ///
    /// Call this when you are done chaining steps. You can then call
    /// [`WorkflowBuilder::build`] to compile the definition.
    ///
    /// # Example
    /// ```ignore
    /// let def = WorkflowBuilder::<MyData>::new()
    ///     .start_with::<A>()
    ///     .then::<B>()
    ///     .end_workflow()
    ///     .build("my-workflow", 1);
    /// ```
    pub fn end_workflow(self) -> WorkflowBuilder<D> {
        self.builder
    }

    /// Shortcut for `end_workflow().build(id, version)`.
    pub fn build(self, id: impl Into<String>, version: u32) -> crate::models::WorkflowDefinition {
        self.builder.build(id, version)
    }
}

impl<D: WorkflowData> ParallelBuilder<D> {
    /// Add a parallel branch.
    pub fn branch(mut self, build_branch: impl FnOnce(&mut WorkflowBuilder<D>)) -> Self {
        let before_count = self.builder.steps.len();
        build_branch(&mut self.builder);
        let after_count = self.builder.steps.len();

        for child_id in before_count..after_count {
            self.builder.add_child(self.container_id, child_id);
        }
        self
    }
}
