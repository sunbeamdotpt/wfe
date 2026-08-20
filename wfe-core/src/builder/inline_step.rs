use async_trait::async_trait;

use crate::models::ExecutionResult;
use crate::traits::step::{StepBody, StepExecutionContext};

type InlineFn = Box<dyn Fn() -> ExecutionResult + Send + Sync>;

/// A step that wraps an inline closure.
pub struct InlineStep {
    body: InlineFn,
}

impl InlineStep {
    /// Create a new inline step from a closure that returns an execution result.
    pub fn new(f: impl Fn() -> ExecutionResult + Send + Sync + 'static) -> Self {
        Self { body: Box::new(f) }
    }
}

impl Default for InlineStep {
    fn default() -> Self {
        Self::new(ExecutionResult::next)
    }
}

#[async_trait]
impl StepBody for InlineStep {
    async fn run(&mut self, _context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        Ok((self.body)())
    }
}
