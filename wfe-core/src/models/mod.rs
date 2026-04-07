pub mod condition;
pub mod error_behavior;
pub mod event;
pub mod execution_error;
pub mod execution_pointer;
pub mod execution_result;
pub mod lifecycle;
pub mod poll_config;
pub mod queue_type;
pub mod scheduled_command;
pub mod schema;
pub mod service;
pub mod status;
pub mod workflow_definition;
pub mod workflow_instance;

pub use condition::{ComparisonOp, FieldComparison, StepCondition};
pub use error_behavior::ErrorBehavior;
pub use event::{Event, EventSubscription};
pub use execution_error::ExecutionError;
pub use execution_pointer::ExecutionPointer;
pub use execution_result::ExecutionResult;
pub use lifecycle::{LifecycleEvent, LifecycleEventType};
pub use poll_config::{HttpMethod, PollCondition, PollEndpointConfig};
pub use queue_type::QueueType;
pub use scheduled_command::{CommandName, ScheduledCommand};
pub use schema::{SchemaType, WorkflowSchema};
pub use service::{
    ReadinessCheck, ReadinessProbe, ServiceDefinition, ServiceEndpoint, ServicePort,
};
pub use status::{PointerStatus, WorkflowStatus};
pub use workflow_definition::{StepOutcome, WorkflowDefinition, WorkflowStep};
pub use workflow_instance::WorkflowInstance;

/// Serde helper for `Option<Duration>` as milliseconds.
pub(crate) mod option_duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => serializer.serialize_some(&(d.as_millis() as u64)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        let millis: Option<u64> = Option::deserialize(deserializer)?;
        Ok(millis.map(Duration::from_millis))
    }
}

/// Serde helper for `Duration` as milliseconds (non-optional).
pub(crate) mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}
