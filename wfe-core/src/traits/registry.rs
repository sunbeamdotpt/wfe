use crate::models::WorkflowDefinition;

/// Registry for workflow definitions with version support.
pub trait WorkflowRegistry: Send + Sync {
    /// Register a workflow definition.
    fn register(&mut self, definition: WorkflowDefinition);
    /// Retrieve a workflow definition by id and optional version.
    fn get_definition(&self, id: &str, version: Option<u32>) -> Option<&WorkflowDefinition>;
    /// Check whether a given definition version is registered.
    fn is_registered(&self, id: &str, version: u32) -> bool;
    /// Remove a definition version from the registry. Returns `true` if it existed.
    fn deregister(&mut self, id: &str, version: u32) -> bool;
    /// Return all registered workflow definitions.
    fn get_all_definitions(&self) -> Vec<&WorkflowDefinition>;
}
