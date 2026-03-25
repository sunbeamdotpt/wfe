/// Generates a test suite for any `PersistenceProvider` implementation.
///
/// The macro takes a factory expression that returns an `impl PersistenceProvider`.
/// The factory expression must be async-compatible (it will be `.await`ed).
///
/// # Example
/// ```ignore
/// persistence_suite!(|| async { InMemoryPersistenceProvider::new() });
/// ```
#[macro_export]
macro_rules! persistence_suite {
    ($factory:expr) => {
        mod persistence_suite {
            use super::*;
            use chrono::{Duration, Utc};
            use $crate::models::{
                Event, EventSubscription, ExecutionError, WorkflowInstance, WorkflowStatus,
            };
            use $crate::traits::{
                EventRepository, PersistenceProvider, SubscriptionRepository, WorkflowRepository,
            };

            #[tokio::test]
            async fn create_new_workflow_generates_id() {
                let provider = ($factory)().await;
                let instance = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
                let id = provider.create_new_workflow(&instance).await.unwrap();
                assert!(!id.is_empty());
            }

            #[tokio::test]
            async fn get_workflow_instance_retrieves_workflow() {
                let provider = ($factory)().await;
                let instance = WorkflowInstance::new("test-wf", 1, serde_json::json!({"x": 1}));
                let id = provider.create_new_workflow(&instance).await.unwrap();
                let retrieved = provider.get_workflow_instance(&id).await.unwrap();
                assert_eq!(retrieved.id, id);
                assert_eq!(retrieved.workflow_definition_id, "test-wf");
                assert_eq!(retrieved.data, serde_json::json!({"x": 1}));
            }

            #[tokio::test]
            async fn persist_workflow_updates_fields() {
                let provider = ($factory)().await;
                let mut instance =
                    WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
                let id = provider.create_new_workflow(&instance).await.unwrap();
                instance.id = id.clone();
                instance.description = Some("updated".to_string());
                instance.status = WorkflowStatus::Suspended;
                provider.persist_workflow(&instance).await.unwrap();

                let retrieved = provider.get_workflow_instance(&id).await.unwrap();
                assert_eq!(retrieved.description.as_deref(), Some("updated"));
                assert_eq!(retrieved.status, WorkflowStatus::Suspended);
            }

            #[tokio::test]
            async fn get_runnable_instances_filters_by_time() {
                let provider = ($factory)().await;

                // Runnable with next_execution in the past
                let mut w1 = WorkflowInstance::new("wf", 1, serde_json::json!({}));
                w1.next_execution = Some(0);
                let id1 = provider.create_new_workflow(&w1).await.unwrap();

                // Runnable with next_execution far in the future
                let mut w2 = WorkflowInstance::new("wf", 1, serde_json::json!({}));
                w2.next_execution = Some(i64::MAX);
                let _id2 = provider.create_new_workflow(&w2).await.unwrap();

                // Suspended workflow
                let mut w3 = WorkflowInstance::new("wf", 1, serde_json::json!({}));
                w3.next_execution = Some(0);
                w3.status = WorkflowStatus::Suspended;
                let id3 = provider.create_new_workflow(&w3).await.unwrap();
                // Need to persist updated status
                w3.id = id3;
                provider.persist_workflow(&w3).await.unwrap();

                let runnable = provider.get_runnable_instances(Utc::now()).await.unwrap();
                assert!(runnable.contains(&id1));
                assert_eq!(runnable.len(), 1);
            }

            #[tokio::test]
            async fn persist_errors_stores_errors() {
                let provider = ($factory)().await;
                let errors = vec![
                    ExecutionError::new("wf-1", "ptr-1", "error one"),
                    ExecutionError::new("wf-1", "ptr-2", "error two"),
                ];
                provider.persist_errors(&errors).await.unwrap();

                // Persist more errors
                let more = vec![ExecutionError::new("wf-2", "ptr-3", "error three")];
                provider.persist_errors(&more).await.unwrap();

                // We can't directly read errors from the trait, but persist_errors should not fail
            }

            #[tokio::test]
            async fn create_and_get_subscription() {
                let provider = ($factory)().await;
                let sub = EventSubscription::new(
                    "wf-1",
                    0,
                    "ptr-1",
                    "order.created",
                    "order-123",
                    Utc::now(),
                );
                let id = provider.create_event_subscription(&sub).await.unwrap();
                let retrieved = provider.get_subscription(&id).await.unwrap();
                assert_eq!(retrieved.id, id);
                assert_eq!(retrieved.event_name, "order.created");
                assert_eq!(retrieved.event_key, "order-123");
            }

            #[tokio::test]
            async fn get_subscriptions_by_event() {
                let provider = ($factory)().await;
                let now = Utc::now();

                let sub1 = EventSubscription::new(
                    "wf-1", 0, "ptr-1", "order.created", "key-A", now,
                );
                provider.create_event_subscription(&sub1).await.unwrap();

                let sub2 = EventSubscription::new(
                    "wf-2", 1, "ptr-2", "order.created", "key-A", now,
                );
                provider.create_event_subscription(&sub2).await.unwrap();

                let sub3 = EventSubscription::new(
                    "wf-3", 0, "ptr-3", "order.created", "key-B", now,
                );
                provider.create_event_subscription(&sub3).await.unwrap();

                let result = provider
                    .get_subscriptions("order.created", "key-A", now + Duration::seconds(1))
                    .await
                    .unwrap();
                assert_eq!(result.len(), 2);
            }

            #[tokio::test]
            async fn terminate_subscription() {
                let provider = ($factory)().await;
                let now = Utc::now();
                let sub = EventSubscription::new(
                    "wf-1", 0, "ptr-1", "evt", "key", now,
                );
                let id = provider.create_event_subscription(&sub).await.unwrap();

                provider.terminate_subscription(&id).await.unwrap();

                // Terminated subscriptions should not appear in get_subscriptions
                let subs = provider
                    .get_subscriptions("evt", "key", now + Duration::seconds(1))
                    .await
                    .unwrap();
                assert!(subs.is_empty());
            }

            #[tokio::test]
            async fn create_and_get_event() {
                let provider = ($factory)().await;
                let event =
                    Event::new("order.created", "order-123", serde_json::json!({"x": 1}));
                let id = provider.create_event(&event).await.unwrap();
                let retrieved = provider.get_event(&id).await.unwrap();
                assert_eq!(retrieved.id, id);
                assert_eq!(retrieved.event_name, "order.created");
                assert_eq!(retrieved.event_data, serde_json::json!({"x": 1}));
            }

            #[tokio::test]
            async fn get_runnable_events() {
                let provider = ($factory)().await;
                let event = Event::new("evt", "key", serde_json::json!(null));
                let id = provider.create_event(&event).await.unwrap();

                let runnable = provider
                    .get_runnable_events(Utc::now() + Duration::seconds(1))
                    .await
                    .unwrap();
                assert!(runnable.contains(&id));
            }

            #[tokio::test]
            async fn mark_event_processed() {
                let provider = ($factory)().await;
                let event = Event::new("evt", "key", serde_json::json!(null));
                let id = provider.create_event(&event).await.unwrap();

                provider.mark_event_processed(&id).await.unwrap();

                let runnable = provider
                    .get_runnable_events(Utc::now() + Duration::seconds(1))
                    .await
                    .unwrap();
                assert!(!runnable.contains(&id));

                let retrieved = provider.get_event(&id).await.unwrap();
                assert!(retrieved.is_processed);
            }

            #[tokio::test]
            async fn concurrent_persist_workflow_no_data_race() {
                let provider = std::sync::Arc::new(($factory)().await);
                let mut handles = Vec::new();

                for i in 0..30 {
                    let p = provider.clone();
                    handles.push(tokio::spawn(async move {
                        let instance = WorkflowInstance::new(
                            format!("wf-{i}"),
                            1,
                            serde_json::json!({"i": i}),
                        );
                        p.create_new_workflow(&instance).await.unwrap()
                    }));
                }

                let mut ids = Vec::new();
                for handle in handles {
                    ids.push(handle.await.unwrap());
                }

                // All 30 should be unique
                let unique: std::collections::HashSet<_> = ids.iter().collect();
                assert_eq!(unique.len(), 30);

                // All should be retrievable
                for id in &ids {
                    let w = provider.get_workflow_instance(id).await.unwrap();
                    assert_eq!(w.id, *id);
                }
            }
        }
    };
}
