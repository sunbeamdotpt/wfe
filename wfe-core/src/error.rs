use thiserror::Error;

#[derive(Debug, Error)]
/// Wfeerror.
pub enum WfeError {
    #[error("Workflow not found: {0}")]
    /// Workflownotfound.
    WorkflowNotFound(String),

    #[error("Workflow definition not found: {id} v{version}")]
    /// Workflow definition was not found.
    DefinitionNotFound {
        /// Definition identifier.
        id: String,
        /// Definition version.
        version: u32,
    },

    #[error("Event not found: {0}")]
    /// Eventnotfound.
    EventNotFound(String),

    #[error("Subscription not found: {0}")]
    /// Subscriptionnotfound.
    SubscriptionNotFound(String),

    #[error("Step not found: {0}")]
    /// Stepnotfound.
    StepNotFound(usize),

    #[error("Lock acquisition failed for: {0}")]
    /// Lockfailed.
    LockFailed(String),

    #[error("Persistence error: {0}")]
    /// Persistence.
    Persistence(String),

    #[error("Serialization error: {0}")]
    /// Serialization.
    Serialization(#[from] serde_json::Error),

    #[error("Step execution error: {0}")]
    /// Stepexecution.
    StepExecution(String),

    #[error("Workflow cancelled")]
    /// Cancelled.
    Cancelled,

    #[error(transparent)]
    /// Other.
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

/// Result.
pub type Result<T> = std::result::Result<T, WfeError>;
