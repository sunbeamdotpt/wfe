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

            // ─── 1.9 name / sequence / root_workflow_id coverage ────────

            #[tokio::test]
            async fn next_definition_sequence_is_monotonic_per_definition() {
                let provider = ($factory)().await;

                // Persistent backends (postgres) keep the sequence counter
                // table across test runs, so we need unique definition ids
                // per test invocation to get deterministic starting values.
                let id_a = format!(
                    "ci-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let id_b = format!("{id_a}-other");

                // First definition counter starts at 1 and increments.
                assert_eq!(provider.next_definition_sequence(&id_a).await.unwrap(), 1);
                assert_eq!(provider.next_definition_sequence(&id_a).await.unwrap(), 2);
                assert_eq!(provider.next_definition_sequence(&id_a).await.unwrap(), 3);

                // Second definition has an independent counter.
                assert_eq!(provider.next_definition_sequence(&id_b).await.unwrap(), 1);
                assert_eq!(provider.next_definition_sequence(&id_b).await.unwrap(), 2);

                // First definition's counter unaffected.
                assert_eq!(provider.next_definition_sequence(&id_a).await.unwrap(), 4);
            }

            #[tokio::test]
            async fn get_workflow_instance_by_name_resolves_human_name() {
                let provider = ($factory)().await;

                let mut w = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
                w.name = "ci-42".into();
                let id = provider.create_new_workflow(&w).await.unwrap();

                // Fetch by human name returns the same row as fetch-by-id.
                let by_name = provider.get_workflow_instance_by_name("ci-42").await.unwrap();
                assert_eq!(by_name.id, id);
                assert_eq!(by_name.name, "ci-42");

                // Nonexistent name surfaces WorkflowNotFound.
                let missing = provider
                    .get_workflow_instance_by_name("no-such-name")
                    .await;
                assert!(missing.is_err());
            }

            #[tokio::test]
            async fn root_workflow_id_persists_across_save_and_load() {
                let provider = ($factory)().await;

                let parent_id = {
                    let mut p = WorkflowInstance::new("parent", 1, serde_json::json!({}));
                    p.name = "parent-1".into();
                    provider.create_new_workflow(&p).await.unwrap()
                };

                let mut child = WorkflowInstance::new("child", 1, serde_json::json!({}));
                child.name = "child-1".into();
                child.root_workflow_id = Some(parent_id.clone());
                let child_id = provider.create_new_workflow(&child).await.unwrap();

                let loaded = provider.get_workflow_instance(&child_id).await.unwrap();
                assert_eq!(loaded.root_workflow_id.as_deref(), Some(parent_id.as_str()));

                // Round-trip through persist_workflow too.
                let mut updated = loaded.clone();
                updated.description = Some("updated".into());
                provider.persist_workflow(&updated).await.unwrap();
                let reloaded = provider.get_workflow_instance(&child_id).await.unwrap();
                assert_eq!(
                    reloaded.root_workflow_id.as_deref(),
                    Some(parent_id.as_str())
                );
                assert_eq!(reloaded.description.as_deref(), Some("updated"));
            }

            // ─── Additional SubscriptionRepository coverage ────────────

            #[tokio::test]
            async fn subscription_token_lifecycle() {
                let provider = ($factory)().await;
                let now = Utc::now();
                let sub =
                    EventSubscription::new("wf-1", 0, "ptr-1", "evt", "key", now);
                let id = provider.create_event_subscription(&sub).await.unwrap();

                // Claim the subscription with a token — returns true on success.
                let claimed = provider
                    .set_subscription_token(&id, "tok-a", "worker-1", now + Duration::seconds(30))
                    .await
                    .unwrap();
                assert!(claimed);

                // A second set_subscription_token with a different token while
                // the first is still held should fail to claim.
                let reclaimed = provider
                    .set_subscription_token(&id, "tok-b", "worker-2", now + Duration::seconds(30))
                    .await
                    .unwrap_or(false);
                assert!(!reclaimed, "token should not be reclaimed while still held");

                // Clearing with the correct token releases the subscription.
                provider.clear_subscription_token(&id, "tok-a").await.unwrap();

                // Now another worker can claim it.
                let re = provider
                    .set_subscription_token(&id, "tok-b", "worker-2", now + Duration::seconds(30))
                    .await
                    .unwrap();
                assert!(re);
            }

            #[tokio::test]
            async fn get_first_open_subscription_returns_unlocked_only() {
                let provider = ($factory)().await;
                let now = Utc::now();

                // Two subscriptions matching the same (event_name, event_key)
                // — the first gets claimed, then get_first_open should return
                // the second.
                let sub1 =
                    EventSubscription::new("wf-1", 0, "p1", "order.created", "k", now);
                let id1 = provider.create_event_subscription(&sub1).await.unwrap();
                let sub2 =
                    EventSubscription::new("wf-2", 0, "p2", "order.created", "k", now);
                let _id2 = provider.create_event_subscription(&sub2).await.unwrap();

                provider
                    .set_subscription_token(&id1, "tok", "w", now + Duration::seconds(30))
                    .await
                    .unwrap();

                let first_open = provider
                    .get_first_open_subscription("order.created", "k", now + Duration::seconds(1))
                    .await
                    .unwrap();
                assert!(first_open.is_some());
                // The open one is the un-claimed wf-2, not the claimed wf-1.
                let open = first_open.unwrap();
                assert_eq!(open.workflow_id, "wf-2");
            }

            #[tokio::test]
            async fn persist_workflow_with_subscriptions_round_trip() {
                let provider = ($factory)().await;
                let mut w = WorkflowInstance::new("sub-wf", 1, serde_json::json!({}));
                let id = provider.create_new_workflow(&w).await.unwrap();
                w.id = id.clone();

                let now = Utc::now();
                let subs = vec![
                    EventSubscription::new(&id, 0, "p-0", "a.evt", "k1", now),
                    EventSubscription::new(&id, 1, "p-1", "b.evt", "k2", now),
                ];
                provider
                    .persist_workflow_with_subscriptions(&w, &subs)
                    .await
                    .unwrap();

                let fetched = provider
                    .get_subscriptions("a.evt", "k1", now + Duration::seconds(1))
                    .await
                    .unwrap();
                assert_eq!(fetched.len(), 1);
                assert_eq!(fetched[0].workflow_id, id);
            }

            // ─── Additional EventRepository coverage ────────────────────

            #[tokio::test]
            async fn mark_event_unprocessed_reverses_processed_flag() {
                let provider = ($factory)().await;
                let event = Event::new("evt", "key", serde_json::json!(null));
                let id = provider.create_event(&event).await.unwrap();

                provider.mark_event_processed(&id).await.unwrap();
                let processed = provider.get_event(&id).await.unwrap();
                assert!(processed.is_processed);

                provider.mark_event_unprocessed(&id).await.unwrap();
                let unprocessed = provider.get_event(&id).await.unwrap();
                assert!(!unprocessed.is_processed);
            }

            #[tokio::test]
            async fn get_events_returns_matching_ids() {
                let provider = ($factory)().await;
                let now = Utc::now();

                let e1 = Event::new("foo.created", "abc", serde_json::json!({}));
                let id1 = provider.create_event(&e1).await.unwrap();
                let e2 = Event::new("foo.created", "xyz", serde_json::json!({}));
                let _id2 = provider.create_event(&e2).await.unwrap();
                let e3 = Event::new("bar.created", "abc", serde_json::json!({}));
                let _id3 = provider.create_event(&e3).await.unwrap();

                let matching = provider
                    .get_events("foo.created", "abc", now + Duration::seconds(1))
                    .await
                    .unwrap();
                assert!(matching.contains(&id1));
                assert_eq!(matching.len(), 1);
            }

            // ─── get_workflow_instances (batch fetch) ─────────────────

            #[tokio::test]
            async fn get_workflow_instances_fetches_multiple_by_id() {
                let provider = ($factory)().await;

                let w1 = WorkflowInstance::new("a", 1, serde_json::json!({}));
                let id1 = provider.create_new_workflow(&w1).await.unwrap();
                let w2 = WorkflowInstance::new("b", 1, serde_json::json!({}));
                let id2 = provider.create_new_workflow(&w2).await.unwrap();
                let w3 = WorkflowInstance::new("c", 1, serde_json::json!({}));
                let id3 = provider.create_new_workflow(&w3).await.unwrap();

                let fetched = provider
                    .get_workflow_instances(&[id1.clone(), id2.clone(), id3.clone()])
                    .await
                    .unwrap();
                assert_eq!(fetched.len(), 3);

                // Missing ids are silently filtered out.
                let partial = provider
                    .get_workflow_instances(&[id1.clone(), "never".into()])
                    .await
                    .unwrap();
                assert_eq!(partial.len(), 1);
                assert_eq!(partial[0].id, id1);
            }

            // ─── WorkflowNotFound on bogus id ─────────────────────────

            #[tokio::test]
            async fn get_workflow_instance_missing_is_workflow_not_found() {
                let provider = ($factory)().await;
                let err = provider
                    .get_workflow_instance("definitely-not-an-id")
                    .await
                    .unwrap_err();
                assert!(matches!(err, $crate::WfeError::WorkflowNotFound(_)));
            }

            // ─── ensure_store_exists idempotency ──────────────────────

            #[tokio::test]
            async fn ensure_store_exists_is_idempotent() {
                let provider = ($factory)().await;
                // Calling twice in a row should not error (schema already there).
                provider.ensure_store_exists().await.unwrap();
                provider.ensure_store_exists().await.unwrap();
            }

            // ─── Execution pointer round-trip ──────────────────────────
            //
            // Pointers carry the bulk of the per-step state and touch the
            // trickiest serialization paths (persistence_data, event_data,
            // scope, children, extension_attributes). Explicitly round-trip
            // one through create → update → fetch to catch marshalling bugs.

            #[tokio::test]
            async fn execution_pointer_round_trip() {
                use $crate::models::{ExecutionPointer, PointerStatus};

                let provider = ($factory)().await;

                let mut instance =
                    WorkflowInstance::new("ptr-test", 1, serde_json::json!({}));
                let mut ptr = ExecutionPointer::new(0);
                ptr.status = PointerStatus::Running;
                ptr.step_name = Some("first".into());
                ptr.persistence_data = Some(serde_json::json!({"cursor": 7}));
                ptr.event_name = Some("order.paid".into());
                ptr.event_key = Some("order-42".into());
                ptr.event_published = false;
                ptr.retry_count = 2;
                ptr.scope = vec!["parent-scope".into()];
                ptr.children = vec!["child-a".into(), "child-b".into()];
                ptr.extension_attributes = {
                    let mut m = std::collections::HashMap::new();
                    m.insert("owner".to_string(), serde_json::json!("alice"));
                    m
                };
                instance.execution_pointers.push(ptr);

                let id = provider.create_new_workflow(&instance).await.unwrap();
                let fetched = provider.get_workflow_instance(&id).await.unwrap();

                assert_eq!(fetched.execution_pointers.len(), 1);
                let out = &fetched.execution_pointers[0];
                assert_eq!(out.status, PointerStatus::Running);
                assert_eq!(out.step_name.as_deref(), Some("first"));
                assert_eq!(
                    out.persistence_data.as_ref().map(|v| v["cursor"].as_u64()),
                    Some(Some(7))
                );
                assert_eq!(out.event_name.as_deref(), Some("order.paid"));
                assert_eq!(out.retry_count, 2);
                assert_eq!(out.scope, vec!["parent-scope".to_string()]);
                assert_eq!(out.children.len(), 2);
                assert_eq!(
                    out.extension_attributes.get("owner"),
                    Some(&serde_json::json!("alice"))
                );
            }

            // ─── ScheduledCommandRepository ────────────────────────────

            #[tokio::test]
            async fn scheduled_commands_round_trip_when_supported() {
                use $crate::models::{CommandName, ScheduledCommand};
                use $crate::traits::ScheduledCommandRepository;

                let provider = ($factory)().await;

                // Some backends (postgres, sqlite) support scheduled
                // commands; others don't. Skip the test cleanly on backends
                // that report no support rather than forcing a hard-coded
                // opt-in list here.
                if !provider.supports_scheduled_commands() {
                    return;
                }

                // Use a unique data payload so the UNIQUE(command_name, data)
                // index doesn't collide with previous runs on persistent
                // backends.
                let unique = format!(
                    "payload-{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                );
                let cmd = ScheduledCommand {
                    command_name: CommandName::ProcessWorkflow,
                    data: unique.clone(),
                    execute_time: 0,
                };
                provider.schedule_command(&cmd).await.unwrap();

                // Double-scheduling the same (command_name, data) must not
                // blow up — the implementation uses ON CONFLICT DO NOTHING
                // semantics so this is idempotent.
                provider.schedule_command(&cmd).await.unwrap();

                // Process due commands at a point well past execute_time.
                let processed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let counter = processed.clone();
                provider
                    .process_commands(
                        Utc::now() + Duration::seconds(1),
                        &|_c: ScheduledCommand| {
                            let counter = counter.clone();
                            Box::pin(async move {
                                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                Ok(())
                            })
                        },
                    )
                    .await
                    .unwrap();
                assert!(
                    processed.load(std::sync::atomic::Ordering::SeqCst) >= 1,
                    "expected at least one scheduled command to be processed"
                );
            }
        }
    };
}
