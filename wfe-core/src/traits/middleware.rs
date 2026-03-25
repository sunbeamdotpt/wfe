use async_trait::async_trait;

use crate::models::{ExecutionResult, WorkflowInstance};
use crate::traits::step::StepExecutionContext;

/// Workflow-level middleware with default no-op implementations.
#[async_trait]
pub trait WorkflowMiddleware: Send + Sync {
    async fn pre_workflow(&self, _instance: &WorkflowInstance) -> crate::Result<()> {
        Ok(())
    }
    async fn post_workflow(&self, _instance: &WorkflowInstance) -> crate::Result<()> {
        Ok(())
    }
}

/// Step-level middleware with default no-op implementations.
#[async_trait]
pub trait StepMiddleware: Send + Sync {
    async fn pre_step(&self, _context: &StepExecutionContext<'_>) -> crate::Result<()> {
        Ok(())
    }
    async fn post_step(
        &self,
        _context: &StepExecutionContext<'_>,
        _result: &ExecutionResult,
    ) -> crate::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ExecutionPointer, ExecutionResult, WorkflowInstance};

    struct NoOpWorkflowMiddleware;
    impl WorkflowMiddleware for NoOpWorkflowMiddleware {}

    struct NoOpStepMiddleware;
    impl StepMiddleware for NoOpStepMiddleware {}

    #[tokio::test]
    async fn workflow_middleware_default_pre_workflow() {
        let mw = NoOpWorkflowMiddleware;
        let instance = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        mw.pre_workflow(&instance).await.unwrap();
    }

    #[tokio::test]
    async fn workflow_middleware_default_post_workflow() {
        let mw = NoOpWorkflowMiddleware;
        let instance = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        mw.post_workflow(&instance).await.unwrap();
    }

    #[tokio::test]
    async fn step_middleware_default_pre_step() {
        use crate::models::WorkflowStep;
        let mw = NoOpStepMiddleware;
        let instance = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        let pointer = ExecutionPointer::new(0);
        let step = WorkflowStep::new(0, "test_step");
        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
        };
        mw.pre_step(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn step_middleware_default_post_step() {
        use crate::models::WorkflowStep;
        let mw = NoOpStepMiddleware;
        let instance = WorkflowInstance::new("wf", 1, serde_json::json!({}));
        let pointer = ExecutionPointer::new(0);
        let step = WorkflowStep::new(0, "test_step");
        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &step,
            workflow: &instance,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
        };
        let result = ExecutionResult::next();
        mw.post_step(&ctx, &result).await.unwrap();
    }
}
