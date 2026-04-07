use async_trait::async_trait;

use crate::models::ExecutionResult;
use crate::models::poll_config::PollEndpointConfig;
use crate::traits::step::{StepBody, StepExecutionContext};

/// A step that polls an external HTTP endpoint until a condition is met.
/// The actual HTTP polling is handled by the executor, not this step.
#[derive(Default)]
pub struct PollEndpointStep {
    pub config: PollEndpointConfig,
}

#[async_trait]
impl StepBody for PollEndpointStep {
    async fn run(&mut self, _context: &StepExecutionContext<'_>) -> crate::Result<ExecutionResult> {
        Ok(ExecutionResult::poll_endpoint(self.config.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ExecutionPointer;
    use crate::models::poll_config::{HttpMethod, PollCondition};
    use crate::primitives::test_helpers::*;
    use std::collections::HashMap;
    use std::time::Duration;

    #[tokio::test]
    async fn returns_poll_config() {
        let config = PollEndpointConfig {
            url: "https://api.example.com/status".into(),
            method: HttpMethod::Get,
            headers: HashMap::new(),
            body: None,
            interval: Duration::from_secs(10),
            timeout: Duration::from_secs(300),
            condition: PollCondition::StatusCode(200),
        };

        let mut step = PollEndpointStep {
            config: config.clone(),
        };
        let pointer = ExecutionPointer::new(0);
        let wf_step = default_step();
        let workflow = default_workflow();
        let ctx = make_context(&pointer, &wf_step, &workflow);

        let result = step.run(&ctx).await.unwrap();
        assert!(!result.proceed);
        assert_eq!(result.poll_endpoint, Some(config));
    }
}
