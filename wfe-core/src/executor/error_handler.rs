use chrono::Utc;

use crate::models::{
    ErrorBehavior, ExecutionPointer, PointerStatus, WorkflowDefinition, WorkflowStatus,
};

/// Outcome of handling a step error.
pub struct ErrorHandlerResult {
    /// New pointers.
    pub new_pointers: Vec<ExecutionPointer>,
    /// If set, the workflow status should be changed to this value.
    pub workflow_status: Option<WorkflowStatus>,
}

/// Handle a step execution error by applying the appropriate ErrorBehavior.
///
/// Updates the pointer in place and returns new pointers and optional workflow status change.
pub fn handle_error(
    _error_msg: &str,
    pointer: &mut ExecutionPointer,
    definition: &WorkflowDefinition,
) -> ErrorHandlerResult {
    let mut new_pointers = Vec::new();
    let mut workflow_status = None;

    // Determine error behavior: step-level override or definition default.
    let step = definition.steps.iter().find(|s| s.id == pointer.step_id);
    let behavior = step
        .and_then(|s| s.error_behavior.clone())
        .unwrap_or_else(|| definition.default_error_behavior.clone());

    match behavior {
        ErrorBehavior::Retry {
            interval,
            max_retries,
        } => {
            if max_retries > 0 && pointer.retry_count >= max_retries {
                // Exceeded max retries, suspend the workflow
                pointer.status = PointerStatus::Failed;
                pointer.active = false;
                workflow_status = Some(WorkflowStatus::Suspended);
                tracing::warn!(
                    retry_count = pointer.retry_count,
                    max_retries,
                    "Max retries exceeded, suspending workflow"
                );
            } else {
                pointer.retry_count += 1;
                pointer.status = PointerStatus::Sleeping;
                pointer.active = true;
                pointer.sleep_until =
                    Some(Utc::now() + chrono::Duration::milliseconds(interval.as_millis() as i64));
            }
        }
        ErrorBehavior::Suspend => {
            pointer.active = false;
            pointer.status = PointerStatus::Failed;
            workflow_status = Some(WorkflowStatus::Suspended);
        }
        ErrorBehavior::Terminate => {
            pointer.active = false;
            pointer.status = PointerStatus::Failed;
            workflow_status = Some(WorkflowStatus::Terminated);
        }
        ErrorBehavior::Compensate => {
            pointer.active = false;
            pointer.status = PointerStatus::Failed;

            if let Some(step) = step
                && let Some(comp_step_id) = step.compensation_step_id
            {
                let mut comp_pointer = ExecutionPointer::new(comp_step_id);
                comp_pointer.step_name = definition
                    .steps
                    .iter()
                    .find(|s| s.id == comp_step_id)
                    .and_then(|s| s.name.clone());
                comp_pointer.predecessor_id = Some(pointer.id.clone());
                comp_pointer.scope = pointer.scope.clone();
                new_pointers.push(comp_pointer);
            }
        }
    }

    ErrorHandlerResult {
        new_pointers,
        workflow_status,
    }
}
