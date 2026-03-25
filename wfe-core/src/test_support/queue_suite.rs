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
                provider
                    .queue_work("a", QueueType::Workflow)
                    .await
                    .unwrap();
                provider
                    .queue_work("b", QueueType::Workflow)
                    .await
                    .unwrap();
                provider
                    .queue_work("c", QueueType::Workflow)
                    .await
                    .unwrap();

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
                assert!(provider
                    .dequeue_work(QueueType::Event)
                    .await
                    .unwrap()
                    .is_none());
                assert!(provider
                    .dequeue_work(QueueType::Workflow)
                    .await
                    .unwrap()
                    .is_none());
            }
        }
    };
}
