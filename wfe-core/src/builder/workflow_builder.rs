use std::collections::HashMap;
use std::marker::PhantomData;

use crate::models::{ExecutionResult, StepOutcome, WorkflowDefinition, WorkflowStep};
use crate::traits::step::{StepBody, WorkflowData};

use super::inline_step::InlineStep;
use super::step_builder::StepBuilder;

/// Type alias for boxed inline step closures.
pub type InlineClosureBox = Box<dyn Fn() -> ExecutionResult + Send + Sync>;

/// Fluent builder for constructing workflow definitions.
///
/// Uses an owned-self pattern: each method consumes and returns the builder,
/// avoiding lifetime issues with mutable borrows.
///
/// # Example
/// ```ignore
/// let def = WorkflowBuilder::<MyData>::new()
///     .start_with::<StepA>()
///     .name("Step A")
///     .then::<StepB>()
///     .name("Step B")
///     .end_workflow()
///     .build("my-workflow", 1);
/// ```
pub struct WorkflowBuilder<D: WorkflowData> {
    /// Steps.
    pub steps: Vec<WorkflowStep>,
    /// Last step.
    pub last_step: Option<usize>,
    /// Inline closures keyed by step id, stored for later registration.
    pub(crate) inline_closures: HashMap<usize, InlineClosureBox>,
    _phantom: PhantomData<D>,
}

impl<D: WorkflowData> WorkflowBuilder<D> {
/// New.
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            last_step: None,
            inline_closures: HashMap::new(),
            _phantom: PhantomData,
        }
    }

    /// Add the first step of the workflow.
    pub fn start_with<S: StepBody + Default + 'static>(mut self) -> StepBuilder<D> {
        let id = self.steps.len();
        let step = WorkflowStep::new(id, std::any::type_name::<S>());
        self.steps.push(step);
        self.last_step = Some(id);
        StepBuilder::new(self, id)
    }

    /// Add a step by type name. Used by container builder closures.
    pub fn add_step(&mut self, step_type: &str) -> usize {
        let id = self.steps.len();
        self.steps.push(WorkflowStep::new(id, step_type));
        id
    }

    /// Add a typed step with an optional name and config.
    /// Convenience for use inside `parallel` branch closures.
    pub fn add_step_typed<S: StepBody + Default + 'static>(
        &mut self,
        name: &str,
        config: Option<serde_json::Value>,
    ) -> usize {
        let id = self.add_step(std::any::type_name::<S>());
        self.steps[id].name = Some(name.to_string());
        if let Some(cfg) = config {
            self.steps[id].step_config = Some(cfg);
        }
        id
    }

    /// Wire an outcome from `from_step` to `to_step`.
    pub fn wire_outcome(
        &mut self,
        from_step: usize,
        to_step: usize,
        value: Option<serde_json::Value>,
    ) {
        if let Some(step) = self.steps.get_mut(from_step) {
            step.outcomes.push(StepOutcome {
                next_step: to_step,
                label: None,
                value,
            });
        }
    }

    /// Add a child step ID to a parent container step.
    pub(crate) fn add_child(&mut self, parent: usize, child: usize) {
        if let Some(step) = self.steps.get_mut(parent) {
            step.children.push(child);
        }
    }

    /// Compile the builder into a WorkflowDefinition.
    pub fn build(self, id: impl Into<String>, version: u32) -> WorkflowDefinition {
        let mut def = WorkflowDefinition::new(id, version);
        def.steps = self.steps;
        // Note: inline closures are dropped here. Use `build_with_closures` to retain them.
        def
    }

    /// Compile the builder into a WorkflowDefinition and return any inline closures
    /// keyed by step id.
    pub fn build_with_closures(
        self,
        id: impl Into<String>,
        version: u32,
    ) -> (WorkflowDefinition, HashMap<usize, InlineClosureBox>) {
        let mut def = WorkflowDefinition::new(id, version);
        def.steps = self.steps;
        (def, self.inline_closures)
    }

    /// Register all inline closures from this builder into the given step registry.
    ///
    /// Each inline closure is registered under a unique key derived from the
    /// `InlineStep` type name and step id.
    pub fn register_inline_steps(
        self,
        registry: &mut crate::executor::StepRegistry,
        id: impl Into<String>,
        version: u32,
    ) -> WorkflowDefinition {
        let mut def = WorkflowDefinition::new(id, version);
        def.steps = self.steps;
        for (step_id, closure) in self.inline_closures {
            let closure = std::sync::Arc::new(closure);
            let key = format!("{}::{step_id}", std::any::type_name::<InlineStep>());
            // Update the step_type so the executor resolves correctly.
            if let Some(step) = def.steps.get_mut(step_id) {
                step.step_type = key.clone();
            }
            let closure = closure.clone();
            registry.register_factory(&key, move || {
                let c = closure.clone();
                Box::new(InlineStep::new(move || (c)()))
            });
        }
        def
    }
}

impl<D: WorkflowData> Default for WorkflowBuilder<D> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ErrorBehavior, ExecutionResult};
    use crate::traits::step::StepExecutionContext;
    use pretty_assertions::assert_eq;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Default, Serialize, Deserialize)]
    struct TestData {
        counter: i32,
    }

    #[derive(Default)]
    struct StepA;

    #[async_trait::async_trait]
    impl StepBody for StepA {
        async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::next())
        }
    }

    #[derive(Default)]
    struct StepB;

    #[async_trait::async_trait]
    impl StepBody for StepB {
        async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::next())
        }
    }

    #[derive(Default)]
    struct StepC;

    #[async_trait::async_trait]
    impl StepBody for StepC {
        async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
            Ok(ExecutionResult::next())
        }
    }

    #[test]
    fn build_empty_workflow() {
        let def = WorkflowBuilder::<TestData>::new().build("empty", 1);
        assert_eq!(def.id, "empty");
        assert_eq!(def.version, 1);
        assert!(def.steps.is_empty());
    }

    #[test]
    fn start_with_adds_first_step() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps.len(), 1);
        assert!(def.steps[0].step_type.contains("StepA"));
    }

    #[test]
    fn then_chains_two_steps_with_outcome() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .then::<StepB>()
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps.len(), 2);
        // Step 0 should have outcome pointing to step 1
        assert_eq!(def.steps[0].outcomes.len(), 1);
        assert_eq!(def.steps[0].outcomes[0].next_step, 1);
    }

    #[test]
    fn then_chains_three_steps() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .then::<StepB>()
            .then::<StepC>()
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps.len(), 3);
        assert_eq!(def.steps[0].outcomes[0].next_step, 1);
        assert_eq!(def.steps[1].outcomes[0].next_step, 2);
        assert!(def.steps[2].outcomes.is_empty());
    }

    #[test]
    fn name_sets_step_name() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .name("First Step")
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps[0].name, Some("First Step".into()));
    }

    #[test]
    fn on_error_sets_behavior() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .on_error(ErrorBehavior::Suspend)
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps[0].error_behavior, Some(ErrorBehavior::Suspend));
    }

    #[test]
    fn if_do_inserts_container_with_children() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .if_do::<StepB>(|b| {
                let id = b.add_step(std::any::type_name::<StepC>());
                b.last_step = Some(id);
            })
            .end_workflow()
            .build("test", 1);

        // Steps: 0=StepA, 1=IfStep, 2=StepC (child)
        // StepA -> IfStep -> (after if)
        assert!(def.steps.len() >= 3);
        // The If step should have StepC as a child
        assert!(def.steps[1].step_type.contains("IfStep"));
        assert!(def.steps[1].children.contains(&2));
    }

    #[test]
    fn while_do_inserts_container() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .while_do::<StepB>(|b| {
                b.add_step(std::any::type_name::<StepC>());
            })
            .end_workflow()
            .build("test", 1);

        assert!(def.steps.len() >= 3);
        assert!(def.steps[1].step_type.contains("WhileStep"));
    }

    #[test]
    fn for_each_inserts_container() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .for_each::<StepB>(|b| {
                b.add_step(std::any::type_name::<StepC>());
            })
            .end_workflow()
            .build("test", 1);

        assert!(def.steps.len() >= 3);
        assert!(def.steps[1].step_type.contains("ForEachStep"));
    }

    #[test]
    fn parallel_creates_branches() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .parallel(|branches| {
                branches
                    .branch(|b| {
                        b.add_step(std::any::type_name::<StepB>());
                    })
                    .branch(|b| {
                        b.add_step(std::any::type_name::<StepC>());
                    })
            })
            .end_workflow()
            .build("test", 1);

        // Steps: 0=StepA, 1=Sequence(parallel container), 2=StepB, 3=StepC
        assert!(def.steps.len() >= 4);
        assert!(def.steps[1].step_type.contains("SequenceStep"));
        assert!(def.steps[1].children.len() >= 2);
    }

    #[test]
    fn saga_with_compensation() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .saga(|b| {
                b.add_step(std::any::type_name::<StepB>());
                b.add_step(std::any::type_name::<StepC>());
            })
            .end_workflow()
            .build("test", 1);

        // Saga container should exist and have children
        assert!(def.steps[1].step_type.contains("SagaContainerStep"));
        assert!(def.steps[1].saga);
        assert!(!def.steps[1].children.is_empty());
    }

    #[test]
    fn compensate_with_sets_compensation_step() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .compensate_with::<StepB>()
            .end_workflow()
            .build("test", 1);

        // Step 0 (StepA) should have compensation pointing to step 1 (StepB)
        assert_eq!(def.steps[0].compensation_step_id, Some(1));
        assert!(def.steps[1].step_type.contains("StepB"));
    }

    #[test]
    fn config_sets_step_config() {
        let cfg = serde_json::json!({"namespace": "ory", "timeout": 30});
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .config(cfg.clone())
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps[0].step_config, Some(cfg));
    }

    #[test]
    fn config_chains_with_name() {
        let cfg = serde_json::json!({"namespace": "data"});
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .name("apply-data")
            .config(cfg.clone())
            .then::<StepB>()
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps[0].name, Some("apply-data".into()));
        assert_eq!(def.steps[0].step_config, Some(cfg));
        assert_eq!(def.steps[0].outcomes[0].next_step, 1);
    }

    #[test]
    fn config_on_multiple_steps_of_same_type() {
        let cfg_a = serde_json::json!({"namespace": "ory"});
        let cfg_b = serde_json::json!({"namespace": "data"});
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .name("apply-ory")
            .config(cfg_a.clone())
            .then::<StepA>()
            .name("apply-data")
            .config(cfg_b.clone())
            .end_workflow()
            .build("test", 1);
        assert_eq!(def.steps[0].step_config, Some(cfg_a));
        assert_eq!(def.steps[1].step_config, Some(cfg_b));
        // Both are StepA
        assert_eq!(def.steps[0].step_type, def.steps[1].step_type);
    }

    #[test]
    fn add_step_typed_sets_name_and_config() {
        let cfg = serde_json::json!({"namespace": "ory"});
        let mut builder = WorkflowBuilder::<TestData>::new();
        let id = builder.add_step_typed::<StepA>("apply-ory", Some(cfg.clone()));
        assert_eq!(builder.steps[id].name, Some("apply-ory".into()));
        assert_eq!(builder.steps[id].step_config, Some(cfg));
        assert!(builder.steps[id].step_type.contains("StepA"));
    }

    #[test]
    fn add_step_typed_without_config() {
        let mut builder = WorkflowBuilder::<TestData>::new();
        let id = builder.add_step_typed::<StepB>("my-step", None);
        assert_eq!(builder.steps[id].name, Some("my-step".into()));
        assert_eq!(builder.steps[id].step_config, None);
    }

    #[test]
    fn wire_outcome_connects_steps() {
        let mut builder = WorkflowBuilder::<TestData>::new();
        let id0 = builder.add_step_typed::<StepA>("first", None);
        let id1 = builder.add_step_typed::<StepB>("second", None);
        builder.wire_outcome(id0, id1, None);
        assert_eq!(builder.steps[id0].outcomes.len(), 1);
        assert_eq!(builder.steps[id0].outcomes[0].next_step, id1);
    }

    #[test]
    fn inline_step_via_then_fn() {
        let def = WorkflowBuilder::<TestData>::new()
            .start_with::<StepA>()
            .then_fn(ExecutionResult::next)
            .end_workflow()
            .build("test", 1);

        assert_eq!(def.steps.len(), 2);
        assert!(def.steps[1].step_type.contains("InlineStep"));
        assert_eq!(def.steps[0].outcomes[0].next_step, 1);
    }
}
