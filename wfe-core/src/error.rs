use thiserror::Error;

#[derive(Debug, Error)]
pub enum WfeError {
    #[error("Workflow not found: {0}")]
    WorkflowNotFound(String),

    #[error("Workflow definition not found: {id} v{version}")]
    DefinitionNotFound { id: String, version: u32 },

    #[error("Event not found: {0}")]
    EventNotFound(String),

    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),

    #[error("Step not found: {0}")]
    StepNotFound(usize),

    #[error("Lock acquisition failed for: {0}")]
    LockFailed(String),

    #[error("Persistence error: {0}")]
    Persistence(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Step execution error: {0}")]
    StepExecution(String),

    #[error("Workflow cancelled")]
    Cancelled,

    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

pub type Result<T> = std::result::Result<T, WfeError>;
