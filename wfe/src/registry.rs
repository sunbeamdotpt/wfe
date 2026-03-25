use std::collections::HashMap;

use wfe_core::models::WorkflowDefinition;
use wfe_core::traits::registry::WorkflowRegistry;

/// Concrete in-memory implementation of `WorkflowRegistry`.
pub struct InMemoryWorkflowRegistry {
    definitions: HashMap<(String, u32), WorkflowDefinition>,
}

impl InMemoryWorkflowRegistry {
    pub fn new() -> Self {
        Self {
            definitions: HashMap::new(),
        }
    }
}

impl Default for InMemoryWorkflowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowRegistry for InMemoryWorkflowRegistry {
    fn register(&mut self, definition: WorkflowDefinition) {
        let key = (definition.id.clone(), definition.version);
        self.definitions.insert(key, definition);
    }

    fn get_definition(&self, id: &str, version: Option<u32>) -> Option<&WorkflowDefinition> {
        match version {
            Some(v) => self.definitions.get(&(id.to_string(), v)),
            None => {
                // Return the definition with the highest version for this id.
                self.definitions
                    .iter()
                    .filter(|((def_id, _), _)| def_id == id)
                    .max_by_key(|((_, v), _)| *v)
                    .map(|(_, def)| def)
            }
        }
    }

    fn is_registered(&self, id: &str, version: u32) -> bool {
        self.definitions.contains_key(&(id.to_string(), version))
    }

    fn deregister(&mut self, id: &str, version: u32) -> bool {
        self.definitions
            .remove(&(id.to_string(), version))
            .is_some()
    }

    fn get_all_definitions(&self) -> Vec<&WorkflowDefinition> {
        self.definitions.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_definition(id: &str, version: u32) -> WorkflowDefinition {
        WorkflowDefinition::new(id, version)
    }

    #[test]
    fn registry_register_and_get() {
        let mut registry = InMemoryWorkflowRegistry::new();
        let def = make_definition("my-workflow", 1);
        registry.register(def);

        let retrieved = registry.get_definition("my-workflow", Some(1));
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "my-workflow");
        assert_eq!(retrieved.unwrap().version, 1);
    }

    #[test]
    fn registry_version_support() {
        let mut registry = InMemoryWorkflowRegistry::new();
        registry.register(make_definition("wf", 1));
        registry.register(make_definition("wf", 2));

        // Get specific versions.
        let v1 = registry.get_definition("wf", Some(1)).unwrap();
        assert_eq!(v1.version, 1);

        let v2 = registry.get_definition("wf", Some(2)).unwrap();
        assert_eq!(v2.version, 2);

        // Get latest (None) returns v2.
        let latest = registry.get_definition("wf", None).unwrap();
        assert_eq!(latest.version, 2);
    }

    #[test]
    fn registry_deregister() {
        let mut registry = InMemoryWorkflowRegistry::new();
        registry.register(make_definition("wf", 1));
        assert!(registry.is_registered("wf", 1));

        let removed = registry.deregister("wf", 1);
        assert!(removed);
        assert!(!registry.is_registered("wf", 1));
        assert!(registry.get_definition("wf", Some(1)).is_none());
    }

    #[test]
    fn registry_get_all_definitions() {
        let mut registry = InMemoryWorkflowRegistry::new();
        registry.register(make_definition("a", 1));
        registry.register(make_definition("b", 1));
        registry.register(make_definition("a", 2));

        let all = registry.get_all_definitions();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn registry_get_nonexistent_returns_none() {
        let registry = InMemoryWorkflowRegistry::new();
        assert!(registry.get_definition("nope", Some(1)).is_none());
        assert!(registry.get_definition("nope", None).is_none());
    }

    #[test]
    fn registry_deregister_nonexistent_returns_false() {
        let mut registry = InMemoryWorkflowRegistry::new();
        assert!(!registry.deregister("nope", 1));
    }
}
