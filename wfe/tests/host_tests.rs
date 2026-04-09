use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use wfe::models::{
    ExecutionResult, PointerStatus, StepOutcome, WorkflowDefinition, WorkflowInstance,
    WorkflowStatus, WorkflowStep,
};
use wfe::traits::WorkflowRepository;
use wfe::traits::search::{Page, SearchFilter, SearchIndex, WorkflowSearchResult};
use wfe::traits::step::{StepBody, StepExecutionContext};
use wfe::{WorkflowHost, WorkflowHostBuilder};

use wfe_core::test_support::{
    InMemoryLifecyclePublisher, InMemoryLockProvider, InMemoryPersistenceProvider,
    InMemoryQueueProvider,
};

// ----- Test steps -----

#[derive(Default)]
struct PassthroughStep;

#[async_trait]
impl StepBody for PassthroughStep {
    async fn run(&mut self, _ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        Ok(ExecutionResult::next())
    }
}

#[derive(Default)]
struct WaitForEventStep;

#[async_trait]
impl StepBody for WaitForEventStep {
    async fn run(&mut self, ctx: &StepExecutionContext<'_>) -> wfe_core::Result<ExecutionResult> {
        // If event data is already set, proceed.
        if ctx.execution_pointer.event_published {
            return Ok(ExecutionResult::next());
        }
        // Otherwise, wait for the event.
        Ok(ExecutionResult::wait_for_event(
            "test-event",
            "test-key",
            Utc::now(),
        ))
    }
}

// ----- Stub SearchIndex for testing -----

#[derive(Debug, Clone)]
struct StubSearchIndex;

#[async_trait]
impl SearchIndex for StubSearchIndex {
    async fn index_workflow(&self, _instance: &WorkflowInstance) -> wfe_core::Result<()> {
        Ok(())
    }
    async fn search(
        &self,
        _terms: &str,
        _skip: u64,
        _take: u64,
        _filters: &[SearchFilter],
    ) -> wfe_core::Result<Page<WorkflowSearchResult>> {
        Ok(Page {
            data: vec![],
            total: 0,
        })
    }
    async fn start(&self) -> wfe_core::Result<()> {
        Ok(())
    }
    async fn stop(&self) -> wfe_core::Result<()> {
        Ok(())
    }
}

// ----- Helpers -----

fn build_host() -> (WorkflowHost, Arc<InMemoryPersistenceProvider>) {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    (host, persistence)
}

fn build_host_with_lifecycle() -> (
    WorkflowHost,
    Arc<InMemoryPersistenceProvider>,
    Arc<InMemoryLifecyclePublisher>,
) {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());
    let lifecycle = Arc::new(InMemoryLifecyclePublisher::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .use_lifecycle(lifecycle.clone() as Arc<dyn wfe_core::traits::LifecyclePublisher>)
        .build()
        .unwrap();

    (host, persistence, lifecycle)
}

fn build_host_with_search() -> (WorkflowHost, Arc<InMemoryPersistenceProvider>) {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());
    let search = Arc::new(StubSearchIndex);

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .use_search(search as Arc<dyn SearchIndex>)
        .build()
        .unwrap();

    (host, persistence)
}

fn build_host_full() -> (
    WorkflowHost,
    Arc<InMemoryPersistenceProvider>,
    Arc<InMemoryLifecyclePublisher>,
) {
    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());
    let lifecycle = Arc::new(InMemoryLifecyclePublisher::new());
    let search = Arc::new(StubSearchIndex);

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence.clone() as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .use_lifecycle(lifecycle.clone() as Arc<dyn wfe_core::traits::LifecyclePublisher>)
        .use_search(search as Arc<dyn SearchIndex>)
        .build()
        .unwrap();

    (host, persistence, lifecycle)
}

fn make_simple_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("simple-workflow", 1);
    let mut step0 = WorkflowStep::new(0, std::any::type_name::<PassthroughStep>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<PassthroughStep>());
    def.steps = vec![step0, step1];
    def
}

fn make_event_definition() -> WorkflowDefinition {
    let mut def = WorkflowDefinition::new("event-workflow", 1);
    let mut step0 = WorkflowStep::new(0, std::any::type_name::<WaitForEventStep>());
    step0.outcomes.push(StepOutcome {
        next_step: 1,
        label: None,
        value: None,
    });
    let step1 = WorkflowStep::new(1, std::any::type_name::<PassthroughStep>());
    def.steps = vec![step0, step1];
    def
}

// ----- Tests -----

#[tokio::test]
async fn host_start_stop() {
    let (host, _) = build_host();
    host.start().await.unwrap();
    // Give background tasks a moment to spawn.
    tokio::time::sleep(Duration::from_millis(50)).await;
    host.stop().await;
}

#[tokio::test]
async fn host_start_workflow() {
    let (host, _) = build_host();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    let id = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    assert!(!id.is_empty());
}

#[tokio::test]
async fn host_start_workflow_unknown_definition() {
    let (host, _) = build_host();
    let result = host
        .start_workflow("nonexistent", 1, serde_json::json!({}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn host_register_and_start_workflow() {
    let (host, _persistence) = build_host();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let id = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Poll until complete or timeout.
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("Workflow did not complete within timeout");
        }
        let instance = host.get_workflow(&id).await.unwrap();
        if instance.status == WorkflowStatus::Complete {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    host.stop().await;
}

#[tokio::test]
async fn sync_runner_completes_simple_workflow() {
    let (host, _) = build_host();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let instance = wfe::run_workflow_sync(
        &host,
        "simple-workflow",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);
    host.stop().await;
}

#[tokio::test]
async fn host_publish_event_resumes_workflow() {
    let (host, _) = build_host();
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let id = host
        .start_workflow("event-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Wait for the workflow to reach WaitingForEvent status on the first pointer.
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            panic!("Workflow pointer did not reach WaitingForEvent");
        }
        let instance = host.get_workflow(&id).await.unwrap();
        let waiting = instance
            .execution_pointers
            .iter()
            .any(|p| p.status == PointerStatus::WaitingForEvent);
        if waiting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Publish the event.
    host.publish_event("test-event", "test-key", serde_json::json!({"done": true}))
        .await
        .unwrap();

    // Wait for workflow to complete.
    let start2 = tokio::time::Instant::now();
    loop {
        if start2.elapsed() > timeout {
            panic!("Workflow did not complete after event");
        }
        let instance = host.get_workflow(&id).await.unwrap();
        if instance.status == WorkflowStatus::Complete {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    host.stop().await;
}

#[tokio::test]
async fn host_suspend_resume() {
    let (host, _) = build_host();
    // Use event workflow so it won't complete immediately.
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let id = host
        .start_workflow("event-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Wait for it to be waiting for event (so it's persisted).
    let timeout = Duration::from_secs(5);
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() > timeout {
            break; // Will try to suspend anyway.
        }
        let instance = host.get_workflow(&id).await.unwrap();
        if instance
            .execution_pointers
            .iter()
            .any(|p| p.status == PointerStatus::WaitingForEvent)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // Suspend.
    let suspended = host.suspend_workflow(&id).await.unwrap();
    assert!(suspended);
    let instance = host.get_workflow(&id).await.unwrap();
    assert_eq!(instance.status, WorkflowStatus::Suspended);

    // Resume.
    let resumed = host.resume_workflow(&id).await.unwrap();
    assert!(resumed);
    let instance = host.get_workflow(&id).await.unwrap();
    assert_eq!(instance.status, WorkflowStatus::Runnable);

    host.stop().await;
}

#[tokio::test]
async fn host_terminate() {
    let (host, _) = build_host();
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let id = host
        .start_workflow("event-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Wait a moment for the workflow to be persisted and processed.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let terminated = host.terminate_workflow(&id).await.unwrap();
    assert!(terminated);
    let instance = host.get_workflow(&id).await.unwrap();
    assert_eq!(instance.status, WorkflowStatus::Terminated);
    assert!(instance.complete_time.is_some());

    host.stop().await;
}

// ----- New tests for coverage gaps -----

// host_builder.rs: use_lifecycle, use_search, Default

#[test]
fn host_builder_default() {
    // Covers WorkflowHostBuilder::default() -> Self::new()
    let _builder = WorkflowHostBuilder::default();
}

#[tokio::test]
async fn host_builder_with_lifecycle() {
    // Covers use_lifecycle() and the executor.with_lifecycle() path in build()
    let (host, _, lifecycle) = build_host_with_lifecycle();

    // Verify lifecycle is accessible.
    assert!(host.lifecycle().is_some());

    // Verify lifecycle publisher was wired in (no events published yet).
    let events = lifecycle.events().await;
    assert!(events.is_empty());
}

#[tokio::test]
async fn host_builder_with_search() {
    // Covers use_search() and the executor.with_search() path in build()
    let (host, _) = build_host_with_search();

    // Start the host to exercise search.start() inside host.start().
    host.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Stop exercises search.stop() inside host.stop().
    host.stop().await;
}

#[tokio::test]
async fn host_builder_with_lifecycle_and_search() {
    // Covers both optional setters together.
    let (host, _, _lifecycle) = build_host_full();

    host.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    host.stop().await;
}

// host.rs: persistence() and lifecycle() accessors

#[tokio::test]
async fn host_persistence_accessor() {
    // Covers host.persistence() -> &Arc<dyn PersistenceProvider>
    let (host, _) = build_host();
    let _p = host.persistence();
}

#[tokio::test]
async fn host_lifecycle_accessor_none() {
    // Covers host.lifecycle() returning None when not configured.
    let (host, _) = build_host();
    assert!(host.lifecycle().is_none());
}

#[tokio::test]
async fn host_lifecycle_accessor_some() {
    // Covers host.lifecycle() returning Some when configured.
    let (host, _, _) = build_host_with_lifecycle();
    assert!(host.lifecycle().is_some());
}

// host.rs: register_workflow via builder closure

#[tokio::test]
async fn host_register_workflow_via_builder() {
    // Covers register_workflow() which takes a builder closure.
    let (host, _) = build_host();
    host.register_step::<PassthroughStep>().await;

    let def = host
        .register_workflow::<serde_json::Value>(
            &|builder| builder.start_with::<PassthroughStep>().end_workflow(),
            "builder-workflow",
            1,
        )
        .await;

    assert_eq!(def.id, "builder-workflow");
    assert_eq!(def.version, 1);
    assert_eq!(def.steps.len(), 1);

    // Should be able to start the workflow now.
    let id = host
        .start_workflow("builder-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    assert!(!id.is_empty());
}

// host.rs: suspend when not runnable returns false

#[tokio::test]
async fn host_suspend_already_suspended_returns_false() {
    let (host, _) = build_host();
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    let id = host
        .start_workflow("event-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Suspend once.
    let suspended = host.suspend_workflow(&id).await.unwrap();
    assert!(suspended);

    // Try to suspend again - already suspended, not Runnable.
    let suspended_again = host.suspend_workflow(&id).await.unwrap();
    assert!(!suspended_again);
}

// host.rs: resume when not suspended returns false

#[tokio::test]
async fn host_resume_when_not_suspended_returns_false() {
    let (host, _) = build_host();
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    let id = host
        .start_workflow("event-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Try to resume a Runnable workflow - should return false.
    let resumed = host.resume_workflow(&id).await.unwrap();
    assert!(!resumed);
}

// host.rs: terminate when already terminated/complete returns false

#[tokio::test]
async fn host_terminate_already_terminated_returns_false() {
    let (host, _) = build_host();
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    let id = host
        .start_workflow("event-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Terminate once.
    let terminated = host.terminate_workflow(&id).await.unwrap();
    assert!(terminated);

    // Try to terminate again.
    let terminated_again = host.terminate_workflow(&id).await.unwrap();
    assert!(!terminated_again);
}

#[tokio::test]
async fn host_terminate_complete_workflow_returns_false() {
    let (host, _) = build_host();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let instance = wfe::run_workflow_sync(
        &host,
        "simple-workflow",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Trying to terminate a completed workflow returns false.
    let terminated = host.terminate_workflow(&instance.id).await.unwrap();
    assert!(!terminated);

    host.stop().await;
}

// purger.rs: stub function call

#[tokio::test]
async fn purger_stub_returns_ok() {
    let persistence = InMemoryPersistenceProvider::new();
    let result = wfe::purge_workflows(&persistence, WorkflowStatus::Complete, Utc::now()).await;
    assert!(result.is_ok());
}

// sync_runner.rs: timeout case

#[tokio::test]
async fn sync_runner_timeout() {
    let (host, _) = build_host();
    let def = make_event_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<WaitForEventStep>().await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    // Use a very short timeout so it will expire while waiting for an event.
    let result = wfe::run_workflow_sync(
        &host,
        "event-workflow",
        1,
        serde_json::json!({}),
        Duration::from_millis(200),
    )
    .await;

    assert!(result.is_err());
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("did not complete"));

    host.stop().await;
}

// registry.rs: Default impl

#[test]
fn registry_default_impl() {
    let _registry = wfe::InMemoryWorkflowRegistry::default();
}

// host.rs: start with search provider exercises search.start() and search.stop()

#[tokio::test]
async fn host_start_stop_with_search() {
    let (host, _) = build_host_with_search();
    host.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    host.stop().await;
}

// host.rs: workflow execution with lifecycle publisher

#[tokio::test]
async fn host_workflow_execution_with_lifecycle() {
    let (host, _, _lifecycle) = build_host_with_lifecycle();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let instance = wfe::run_workflow_sync(
        &host,
        "simple-workflow",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Verify the lifecycle publisher is wired into the host.
    assert!(host.lifecycle().is_some());

    host.stop().await;
}

// host.rs: event publishing with no matching subscriptions
// Covers the process_event path where subscriptions list is empty.

#[tokio::test]
async fn host_publish_event_no_matching_subscriptions() {
    let (host, _) = build_host();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    // Publish an event that no workflow is waiting for.
    let result = host
        .publish_event("no-match-event", "no-match-key", serde_json::json!({}))
        .await;
    assert!(result.is_ok());

    // Give the event consumer time to process.
    tokio::time::sleep(Duration::from_millis(200)).await;

    host.stop().await;
}

// host.rs: full lifecycle with search + lifecycle enabled

#[tokio::test]
async fn host_full_workflow_with_search_and_lifecycle() {
    let (host, _, _lifecycle) = build_host_full();
    let def = make_simple_definition();
    host.register_workflow_definition(def).await;
    host.register_step::<PassthroughStep>().await;

    host.start().await.unwrap();

    let instance = wfe::run_workflow_sync(
        &host,
        "simple-workflow",
        1,
        serde_json::json!({}),
        Duration::from_secs(5),
    )
    .await
    .unwrap();

    assert_eq!(instance.status, WorkflowStatus::Complete);
    host.stop().await;
}

// ─── 1.9 name / resolve path tests ──────────────────────────────────
//
// Every test below pins a 1.9 behavior introduced by the shift from
// UUID-only addressing to human-friendly names: auto-sequencing,
// caller-supplied overrides, whitespace rejection, and transparent
// name-or-UUID lookup across Get/Suspend/Resume/Terminate.

#[tokio::test]
async fn start_workflow_auto_assigns_sequenced_name() {
    let (host, persistence) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    // Three consecutive runs of the same definition should produce
    // monotonically incrementing `{definition_id}-N` names.
    let id_a = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    let id_b = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    let id_c = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    let a = persistence.get_workflow_instance(&id_a).await.unwrap();
    let b = persistence.get_workflow_instance(&id_b).await.unwrap();
    let c = persistence.get_workflow_instance(&id_c).await.unwrap();

    assert_eq!(a.name, "simple-workflow-1");
    assert_eq!(b.name, "simple-workflow-2");
    assert_eq!(c.name, "simple-workflow-3");

    // UUIDs still unique — names are a parallel index, not a replacement.
    assert_ne!(a.id, b.id);
    assert_ne!(b.id, c.id);
}

#[tokio::test]
async fn start_workflow_with_name_uses_explicit_override() {
    let (host, persistence) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let id = host
        .start_workflow_with_name(
            "simple-workflow",
            1,
            serde_json::json!({}),
            Some("ci-1.9.0-release".into()),
        )
        .await
        .unwrap();

    let instance = persistence.get_workflow_instance(&id).await.unwrap();
    assert_eq!(instance.name, "ci-1.9.0-release");
}

#[tokio::test]
async fn start_workflow_with_empty_name_override_is_rejected() {
    let (host, _) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    // A whitespace-only override is treated as empty — rejected so the
    // UNIQUE index can't get "" or "   ".
    let err = host
        .start_workflow_with_name(
            "simple-workflow",
            1,
            serde_json::json!({}),
            Some("   ".into()),
        )
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("non-empty"),
        "expected non-empty rejection, got: {msg}"
    );
}

#[tokio::test]
async fn get_workflow_accepts_uuid_and_name_interchangeably() {
    let (host, _) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let uuid = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Lookup by UUID (primary key) works.
    let by_uuid = host.get_workflow(&uuid).await.unwrap();
    // Lookup by the auto-assigned human name also works.
    let by_name = host.get_workflow(&by_uuid.name).await.unwrap();
    // Both return the same instance.
    assert_eq!(by_uuid.id, by_name.id);
    assert_eq!(by_uuid.name, by_name.name);
}

#[tokio::test]
async fn get_workflow_nonexistent_returns_error() {
    let (host, _) = build_host();
    let err = host
        .get_workflow("neither-a-uuid-nor-a-name")
        .await
        .unwrap_err();
    assert!(matches!(err, wfe_core::WfeError::WorkflowNotFound(_)));
}

#[tokio::test]
async fn resolve_workflow_id_returns_canonical_uuid() {
    let (host, _) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let uuid = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    let instance = host.get_workflow(&uuid).await.unwrap();

    // Identity case.
    let resolved_from_uuid = host.resolve_workflow_id(&uuid).await.unwrap();
    assert_eq!(resolved_from_uuid, uuid);

    // Name → UUID case.
    let resolved_from_name = host.resolve_workflow_id(&instance.name).await.unwrap();
    assert_eq!(resolved_from_name, uuid);
}

#[tokio::test]
async fn suspend_and_resume_accept_name_in_addition_to_uuid() {
    let (host, persistence, _lifecycle) = build_host_with_lifecycle();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let uuid = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    let name = persistence.get_workflow_instance(&uuid).await.unwrap().name;

    // Suspend via human name.
    let suspended = host.suspend_workflow(&name).await.unwrap();
    assert!(suspended);
    let after_suspend = persistence.get_workflow_instance(&uuid).await.unwrap();
    assert_eq!(after_suspend.status, WorkflowStatus::Suspended);

    // Resume via UUID (to prove both paths still work).
    let resumed = host.resume_workflow(&uuid).await.unwrap();
    assert!(resumed);
    let after_resume = persistence.get_workflow_instance(&uuid).await.unwrap();
    assert_eq!(after_resume.status, WorkflowStatus::Runnable);
}

#[tokio::test]
async fn terminate_workflow_via_name() {
    let (host, persistence, _lifecycle) = build_host_with_lifecycle();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let uuid = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();
    let name = persistence.get_workflow_instance(&uuid).await.unwrap().name;

    let terminated = host.terminate_workflow(&name).await.unwrap();
    assert!(terminated);
    let final_state = persistence.get_workflow_instance(&uuid).await.unwrap();
    assert_eq!(final_state.status, WorkflowStatus::Terminated);
    // Terminating a second time is a no-op.
    assert!(!host.terminate_workflow(&name).await.unwrap());
}

#[tokio::test]
async fn suspend_nonrunnable_workflow_returns_false() {
    let (host, persistence) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let uuid = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Terminate first, then try to suspend — suspend on a terminated
    // workflow should return false, not error.
    host.terminate_workflow(&uuid).await.unwrap();
    let suspended = host.suspend_workflow(&uuid).await.unwrap();
    assert!(!suspended);

    let state = persistence.get_workflow_instance(&uuid).await.unwrap();
    assert_eq!(state.status, WorkflowStatus::Terminated);
}

#[tokio::test]
async fn resume_non_suspended_workflow_returns_false() {
    let (host, _) = build_host();
    host.register_workflow_definition(make_simple_definition())
        .await;
    host.register_step::<PassthroughStep>().await;

    let uuid = host
        .start_workflow("simple-workflow", 1, serde_json::json!({}))
        .await
        .unwrap();

    // Workflow is Runnable, not Suspended — resume should no-op return false.
    let resumed = host.resume_workflow(&uuid).await.unwrap();
    assert!(!resumed);
}
