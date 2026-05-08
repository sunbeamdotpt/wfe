use chrono::Utc;

use crate::models::{
    EventSubscription, ExecutionPointer, ExecutionResult, PointerStatus, WorkflowDefinition,
};

/// Outcome of processing an ExecutionResult: new pointers, subscriptions, and output data.
pub struct ProcessResult {
    /// New pointers.
    pub new_pointers: Vec<ExecutionPointer>,
    /// Subscriptions.
    pub subscriptions: Vec<EventSubscription>,
    /// Output data to merge into workflow.data (from step's output_data field).
    pub output_data: Option<serde_json::Value>,
}

/// Process an ExecutionResult and update the pointer accordingly.
///
/// Returns new pointers to add and subscriptions to create.
pub fn process_result(
    result: &ExecutionResult,
    pointer: &mut ExecutionPointer,
    definition: &WorkflowDefinition,
    workflow_id: &str,
) -> ProcessResult {
    let mut new_pointers = Vec::new();
    let mut subscriptions = Vec::new();

    if result.proceed {
        // Step completed - mark pointer done.
        pointer.active = false;
        pointer.status = PointerStatus::Complete;
        pointer.end_time = Some(Utc::now());

        // Determine the next step via outcomes.
        let step = definition.steps.iter().find(|s| s.id == pointer.step_id);
        if let Some(step) = step {
            let next_step_id = find_next_step(step, &result.outcome_value);
            if let Some(next_id) = next_step_id {
                let mut next_pointer = ExecutionPointer::new(next_id);
                next_pointer.step_name = definition
                    .steps
                    .iter()
                    .find(|s| s.id == next_id)
                    .and_then(|s| s.name.clone());
                next_pointer.predecessor_id = Some(pointer.id.clone());
                next_pointer.scope = pointer.scope.clone();
                new_pointers.push(next_pointer);
            }
        }

        if let Some(outcome_value) = &result.outcome_value {
            pointer.outcome = Some(outcome_value.clone());
        }
    } else if let Some(branch_values) = &result.branch_values {
        // Branch: create child pointers for each value.
        pointer.status = PointerStatus::Running;
        pointer.persistence_data = result.persistence_data.clone();

        let step = definition.steps.iter().find(|s| s.id == pointer.step_id);
        let child_step_ids: Vec<usize> = step.map(|s| s.children.clone()).unwrap_or_default();

        let mut child_scope = pointer.scope.clone();
        child_scope.push(pointer.id.clone());

        for value in branch_values {
            for &child_step_id in &child_step_ids {
                let mut child_pointer = ExecutionPointer::new(child_step_id);
                child_pointer.step_name = definition
                    .steps
                    .iter()
                    .find(|s| s.id == child_step_id)
                    .and_then(|s| s.name.clone());
                child_pointer.context_item = Some(value.clone());
                child_pointer.scope = child_scope.clone();
                child_pointer.predecessor_id = Some(pointer.id.clone());
                pointer.children.push(child_pointer.id.clone());
                new_pointers.push(child_pointer);
            }
        }
    } else if result.event_name.is_some() {
        // Wait for event.
        pointer.status = PointerStatus::WaitingForEvent;
        pointer.active = false;
        pointer.event_name = result.event_name.clone();
        pointer.event_key = result.event_key.clone();

        if let (Some(event_name), Some(event_key)) = (&result.event_name, &result.event_key) {
            let as_of = result.event_as_of.unwrap_or_else(Utc::now);
            let sub = EventSubscription::new(
                workflow_id,
                pointer.step_id,
                pointer.id.as_str(),
                event_name.as_str(),
                event_key.as_str(),
                as_of,
            );
            subscriptions.push(sub);
        }
    } else if result.sleep_for.is_some() {
        // Sleep.
        pointer.status = PointerStatus::Sleeping;
        pointer.active = true;
        if let Some(duration) = result.sleep_for {
            pointer.sleep_until =
                Some(Utc::now() + chrono::Duration::milliseconds(duration.as_millis() as i64));
        }
        pointer.persistence_data = result.persistence_data.clone();
    } else if let Some(poll_config) = &result.poll_endpoint {
        // Poll endpoint: store config and sleep for the interval.
        pointer.status = PointerStatus::Sleeping;
        pointer.active = true;
        pointer.sleep_until = Some(
            Utc::now() + chrono::Duration::milliseconds(poll_config.interval.as_millis() as i64),
        );
        pointer.persistence_data = result.persistence_data.clone();
    } else if result.persistence_data.is_some() {
        // Persist: keep pointer active with data.
        pointer.active = true;
        pointer.status = PointerStatus::Running;
        pointer.persistence_data = result.persistence_data.clone();
    }

    ProcessResult {
        new_pointers,
        subscriptions,
        output_data: result.output_data.clone(),
    }
}

/// Find the next step ID based on the step's outcomes and the result's outcome value.
fn find_next_step(
    step: &crate::models::WorkflowStep,
    outcome_value: &Option<serde_json::Value>,
) -> Option<usize> {
    if step.outcomes.is_empty() {
        return None;
    }

    if let Some(value) = outcome_value {
        // Try to match a specific outcome value.
        for outcome in &step.outcomes {
            if let Some(ov) = &outcome.value
                && ov == value
            {
                return Some(outcome.next_step);
            }
        }
    }

    // Fall back to the default outcome (value == None).
    for outcome in &step.outcomes {
        if outcome.value.is_none() {
            return Some(outcome.next_step);
        }
    }

    // If no default, take the first outcome.
    step.outcomes.first().map(|o| o.next_step)
}
