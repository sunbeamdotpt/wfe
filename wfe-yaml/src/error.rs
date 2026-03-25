#[derive(Debug, thiserror::Error)]
pub enum YamlWorkflowError {
    #[error("YAML parse error: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("Interpolation error: unresolved variable '{0}'")]
    UnresolvedVariable(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Compilation error: {0}")]
    Compilation(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
