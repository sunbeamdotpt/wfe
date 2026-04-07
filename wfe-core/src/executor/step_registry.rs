use std::collections::HashMap;

use crate::traits::StepBody;

/// Registry of step factories keyed by type name.
pub struct StepRegistry {
    factories: HashMap<String, Box<dyn Fn() -> Box<dyn StepBody> + Send + Sync>>,
}

impl StepRegistry {
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a step type using its full type name as the key.
    pub fn register<S: StepBody + Default + 'static>(&mut self) {
        let key = std::any::type_name::<S>().to_string();
        self.factories
            .insert(key, Box::new(|| Box::new(S::default())));
    }

    /// Register a step factory with an explicit key and factory function.
    pub fn register_factory(
        &mut self,
        key: &str,
        factory: impl Fn() -> Box<dyn StepBody> + Send + Sync + 'static,
    ) {
        self.factories.insert(key.to_string(), Box::new(factory));
    }

    /// Resolve a step instance by type name.
    pub fn resolve(&self, step_type: &str) -> Option<Box<dyn StepBody>> {
        self.factories.get(step_type).map(|f| f())
    }
}

impl Default for StepRegistry {
    fn default() -> Self {
        Self::new()
    }
}
