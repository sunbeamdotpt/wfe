/// Generates a test suite for any `QueueProvider` implementation.
///
/// The macro takes a factory expression that returns an `impl QueueProvider`.
///
/// # Example
/// ```ignore
/// queue_suite!(|| async { InMemoryQueueProvider::new() });
/// ```
#[macro_export]
macro_rules! queue_suite {
    ($factory:expr) => {
        mod queue_suite {
            use super::*;
            use $crate::models::QueueType;
            use $crate::traits::QueueProvider;

            #[tokio::test]
            async fn enqueue_dequeue_fifo() {
                let provider = ($factory)().await;
                provider.queue_work("a", QueueType::Workflow).await.unwrap();
                provider.queue_work("b", QueueType::Workflow).await.unwrap();
                provider.queue_work("c", QueueType::Workflow).await.unwrap();

                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("a")
                );
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("b")
                );
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("c")
                );
            }

            #[tokio::test]
            async fn dequeue_empty_returns_none() {
                let provider = ($factory)().await;
                let result = provider.dequeue_work(QueueType::Workflow).await.unwrap();
                assert!(result.is_none());
            }

            #[tokio::test]
            async fn multiple_queue_types_independent() {
                let provider = ($factory)().await;
                provider
                    .queue_work("wf-1", QueueType::Workflow)
                    .await
                    .unwrap();
                provider
                    .queue_work("evt-1", QueueType::Event)
                    .await
                    .unwrap();

                // Dequeue from Event queue should get evt-1, not wf-1
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Event)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("evt-1")
                );
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("wf-1")
                );

                // Both should now be empty
                assert!(
                    provider
                        .dequeue_work(QueueType::Event)
                        .await
                        .unwrap()
                        .is_none()
                );
                assert!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .is_none()
                );
            }

            #[tokio::test]
            async fn index_queue_type_is_isolated() {
                let provider = ($factory)().await;
                provider
                    .queue_work("idx-1", QueueType::Index)
                    .await
                    .unwrap();
                provider
                    .queue_work("idx-2", QueueType::Index)
                    .await
                    .unwrap();
                provider
                    .queue_work("wf-1", QueueType::Workflow)
                    .await
                    .unwrap();

                // Index queue drains in FIFO order...
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Index)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("idx-1")
                );
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Index)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("idx-2")
                );
                // ...and doesn't disturb the Workflow queue.
                assert_eq!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .as_deref(),
                    Some("wf-1")
                );
            }

            #[tokio::test]
            async fn start_and_stop_lifecycle_are_idempotent() {
                let provider = ($factory)().await;
                // Both start and stop should be no-ops that can be called
                // multiple times without error regardless of backend.
                provider.start().await.unwrap();
                provider.start().await.unwrap();
                provider.stop().await.unwrap();
                provider.stop().await.unwrap();
            }

            #[tokio::test]
            async fn is_dequeue_blocking_is_stable() {
                let provider = ($factory)().await;
                // Pure property — just make sure it doesn't panic and is
                // consistent between calls. Different backends return
                // different values; we only care the call works.
                let a = provider.is_dequeue_blocking();
                let b = provider.is_dequeue_blocking();
                assert_eq!(a, b);
            }

            #[tokio::test]
            async fn enqueue_many_then_drain() {
                let provider = ($factory)().await;
                for i in 0..20u32 {
                    provider
                        .queue_work(&format!("item-{i}"), QueueType::Workflow)
                        .await
                        .unwrap();
                }

                for i in 0..20u32 {
                    let got = provider.dequeue_work(QueueType::Workflow).await.unwrap();
                    assert_eq!(got.as_deref(), Some(format!("item-{i}").as_str()));
                }

                assert!(
                    provider
                        .dequeue_work(QueueType::Workflow)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        }
    };
}
