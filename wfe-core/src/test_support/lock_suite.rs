/// Generates a test suite for any `DistributedLockProvider` implementation.
///
/// The macro takes a factory expression that returns an `impl DistributedLockProvider`.
///
/// # Example
/// ```ignore
/// lock_suite!(|| async { InMemoryLockProvider::new() });
/// ```
#[macro_export]
macro_rules! lock_suite {
    ($factory:expr) => {
        mod lock_suite {
            use super::*;
            use $crate::traits::DistributedLockProvider;

            #[tokio::test]
            async fn acquire_lock_succeeds() {
                let provider = ($factory)().await;
                let acquired = provider.acquire_lock("resource-1").await.unwrap();
                assert!(acquired);
            }

            #[tokio::test]
            async fn double_acquire_fails() {
                let provider = ($factory)().await;
                let first = provider.acquire_lock("resource-1").await.unwrap();
                assert!(first);
                let second = provider.acquire_lock("resource-1").await.unwrap();
                assert!(!second);
            }

            #[tokio::test]
            async fn release_then_reacquire() {
                let provider = ($factory)().await;
                provider.acquire_lock("resource-1").await.unwrap();
                provider.release_lock("resource-1").await.unwrap();
                let reacquired = provider.acquire_lock("resource-1").await.unwrap();
                assert!(reacquired);
            }

            #[tokio::test]
            async fn release_nonexistent_ok() {
                let provider = ($factory)().await;
                // Should not error even if lock was never acquired
                provider.release_lock("nonexistent").await.unwrap();
            }

            #[tokio::test]
            async fn different_resources_are_independent() {
                let provider = ($factory)().await;
                assert!(provider.acquire_lock("resource-a").await.unwrap());
                // Different resource id doesn't block on the first.
                assert!(provider.acquire_lock("resource-b").await.unwrap());
                // Now trying to reacquire either fails while held.
                assert!(!provider.acquire_lock("resource-a").await.unwrap());
                assert!(!provider.acquire_lock("resource-b").await.unwrap());
                provider.release_lock("resource-a").await.unwrap();
                provider.release_lock("resource-b").await.unwrap();
            }

            #[tokio::test]
            async fn start_and_stop_lifecycle_are_idempotent() {
                let provider = ($factory)().await;
                provider.start().await.unwrap();
                provider.start().await.unwrap();
                provider.stop().await.unwrap();
                provider.stop().await.unwrap();
            }

            #[tokio::test]
            async fn acquire_release_acquire_roundtrip() {
                let provider = ($factory)().await;
                for _ in 0..5 {
                    assert!(provider.acquire_lock("cycling").await.unwrap());
                    provider.release_lock("cycling").await.unwrap();
                }
                assert!(provider.acquire_lock("cycling").await.unwrap());
                assert!(!provider.acquire_lock("cycling").await.unwrap());
            }
        }
    };
}
