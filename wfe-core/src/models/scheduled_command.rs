use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandName {
    ProcessWorkflow,
    ProcessEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledCommand {
    pub command_name: CommandName,
    pub data: String,
    /// Epoch milliseconds when the command should execute.
    pub execute_time: i64,
}

impl ScheduledCommand {
    pub fn process_workflow(workflow_id: impl Into<String>, execute_time: i64) -> Self {
        Self {
            command_name: CommandName::ProcessWorkflow,
            data: workflow_id.into(),
            execute_time,
        }
    }

    pub fn process_event(event_id: impl Into<String>, execute_time: i64) -> Self {
        Self {
            command_name: CommandName::ProcessEvent,
            data: event_id.into(),
            execute_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn process_workflow_factory() {
        let cmd = ScheduledCommand::process_workflow("wf-123", 1000);
        assert_eq!(cmd.command_name, CommandName::ProcessWorkflow);
        assert_eq!(cmd.data, "wf-123");
        assert_eq!(cmd.execute_time, 1000);
    }

    #[test]
    fn process_event_factory() {
        let cmd = ScheduledCommand::process_event("evt-456", 2000);
        assert_eq!(cmd.command_name, CommandName::ProcessEvent);
        assert_eq!(cmd.data, "evt-456");
        assert_eq!(cmd.execute_time, 2000);
    }

    #[test]
    fn serde_round_trip() {
        let cmd = ScheduledCommand::process_workflow("wf-1", 500);
        let json = serde_json::to_string(&cmd).unwrap();
        let deserialized: ScheduledCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd.command_name, deserialized.command_name);
        assert_eq!(cmd.data, deserialized.data);
        assert_eq!(cmd.execute_time, deserialized.execute_time);
    }
}
