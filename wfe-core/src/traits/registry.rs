use crate::models::WorkflowDefinition;

/// Registry for workflow definitions with version support.
pub trait WorkflowRegistry: Send + Sync {
    fn register(&mut self, definition: WorkflowDefinition);
    fn get_definition(&self, id: &str, version: Option<u32>) -> Option<&WorkflowDefinition>;
    fn is_registered(&self, id: &str, version: u32) -> bool;
    fn deregister(&mut self, id: &str, version: u32) -> bool;
    fn get_all_definitions(&self) -> Vec<&WorkflowDefinition>;
}
