#[derive(Debug, thiserror::Error)]
/// Yamlworkflowerror.
pub enum YamlWorkflowError {
    #[error("YAML parse error: {0}")]
    /// Parse.
    Parse(#[from] serde_yaml::Error),
    #[error("Interpolation error: unresolved variable '{0}'")]
    /// Unresolvedvariable.
    UnresolvedVariable(String),
    #[error("Validation error: {0}")]
    /// Validation.
    Validation(String),
    #[error("Compilation error: {0}")]
    /// Compilation.
    Compilation(String),
    #[error("IO error: {0}")]
    /// Io.
    Io(#[from] std::io::Error),
}
