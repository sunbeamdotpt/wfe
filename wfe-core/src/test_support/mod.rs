pub mod fixtures;
pub mod in_memory_lifecycle;
pub mod in_memory_lock;
pub mod in_memory_persistence;
pub mod in_memory_queue;

// Test suite macros (exported via #[macro_export] at crate level)
mod lock_suite;
mod persistence_suite;
mod queue_suite;

pub use fixtures::*;
pub use in_memory_lifecycle::InMemoryLifecyclePublisher;
pub use in_memory_lock::InMemoryLockProvider;
pub use in_memory_persistence::InMemoryPersistenceProvider;
pub use in_memory_queue::InMemoryQueueProvider;

// Run the test suites against the in-memory providers.
#[cfg(test)]
mod tests {
    use super::*;

    crate::persistence_suite!(|| async { InMemoryPersistenceProvider::new() });
    crate::lock_suite!(|| async { InMemoryLockProvider::new() });
    crate::queue_suite!(|| async { InMemoryQueueProvider::new() });
}
