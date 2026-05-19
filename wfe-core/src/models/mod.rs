//! Core data models for workflows, execution pointers, events, and results.
//!
//! These types are the building blocks of the WFE runtime. Most are serialized
//! to JSON when persisted or sent over the wire.

/// OCI-compatible artifact references and blobs.
pub mod artifact;
/// Step condition evaluation (used by `if_do` and `while_do`).
pub mod condition;
/// Error handling behavior for failed steps.
pub mod error_behavior;
/// External events and subscriptions.
pub mod event;
/// Errors that occur during step execution.
pub mod execution_error;
/// The runtime pointer tracking a step's execution state.
pub mod execution_pointer;
/// The result returned by a step, controlling flow and persistence.
pub mod execution_result;
/// Lifecycle event types (started, completed, failed, etc.).
pub mod lifecycle;
/// HTTP polling configuration for `PollEndpointStep`.
pub mod poll_config;
/// Work queue classification (workflow vs event queue).
pub mod queue_type;
/// Commands scheduled for future execution.
pub mod scheduled_command;
/// JSON Schema support for workflow definitions.
pub mod schema;
/// Service definitions and readiness probes.
pub mod service;
/// Workflow and pointer status enums.
pub mod status;
/// The static workflow definition (steps, outcomes, metadata).
pub mod workflow_definition;
/// The mutable runtime instance of a workflow.
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
pub use workflow_definition::{SharedVolume, StepOutcome, WorkflowDefinition, WorkflowStep};
pub use workflow_instance::WorkflowInstance;
pub use artifact::{
    ArtifactBlob, ArtifactRef, ARTIFACT_REF_KEY, MEDIA_TYPE_OCI_LAYER_GZIP,
    MEDIA_TYPE_OCI_LAYER_TAR, artifact_ref_value, is_artifact_ref, parse_artifact_ref,
};

/// Serde helper for `Option<Duration>` as milliseconds.
pub(crate) mod option_duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

/// Serialize.
    pub fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(d) => serializer.serialize_some(&(d.as_millis() as u64)),
            None => serializer.serialize_none(),
        }
    }

/// Deserialize.
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

/// Serialize.
    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(duration.as_millis() as u64)
    }

/// Deserialize.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}
