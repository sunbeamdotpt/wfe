#![cfg(feature = "testing")]

use std::sync::Arc;
use std::time::Duration;

use sdk::testing::Nats;
use testcontainers::{ContainerAsync, GenericImage};
use wfe_core::models::{LifecycleEvent, LifecycleEventType, QueueType};
use wfe_core::traits::{DistributedLockProvider, LifecyclePublisher, QueueProvider};
use wfe_nats::{NatsConfig, NatsLifecyclePublisher, NatsLockProvider, NatsQueueProvider};

async fn start_nats() -> (ContainerAsync<GenericImage>, String) {
    let container = Nats::default()
        .publish_port()
        .start()
        .await
        .expect("nats should start");
    let url = Nats::url(&container)
        .await
        .expect("nats url should resolve");
    (container, url)
}

fn test_config(url: String) -> NatsConfig {
    NatsConfig {
        url,
        prefix: format!("wfe-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    }
}

#[tokio::test]
async fn queue_enqueue_dequeue_fifo() {
    let (_container, url) = start_nats().await;
    let config = test_config(url);
    let provider = NatsQueueProvider::new(config).await.unwrap();
    provider.start().await.unwrap();

    provider.queue_work("a", QueueType::Workflow).await.unwrap();
    provider.queue_work("b", QueueType::Workflow).await.unwrap();
    provider.queue_work("c", QueueType::Workflow).await.unwrap();

    // Give the push consumer time to deliver messages to the internal channel.
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(provider.dequeue_work(QueueType::Workflow).await.unwrap().as_deref(), Some("a"));
    assert_eq!(provider.dequeue_work(QueueType::Workflow).await.unwrap().as_deref(), Some("b"));
    assert_eq!(provider.dequeue_work(QueueType::Workflow).await.unwrap().as_deref(), Some("c"));
}

#[tokio::test]
async fn queue_multiple_queue_types_independent() {
    let (_container, url) = start_nats().await;
    let config = test_config(url);
    let provider = NatsQueueProvider::new(config).await.unwrap();
    provider.start().await.unwrap();

    provider.queue_work("wf-1", QueueType::Workflow).await.unwrap();
    provider.queue_work("evt-1", QueueType::Event).await.unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(provider.dequeue_work(QueueType::Event).await.unwrap().as_deref(), Some("evt-1"));
    assert_eq!(provider.dequeue_work(QueueType::Workflow).await.unwrap().as_deref(), Some("wf-1"));
    assert!(provider.dequeue_work(QueueType::Event).await.unwrap().is_none());
    assert!(provider.dequeue_work(QueueType::Workflow).await.unwrap().is_none());
}

#[tokio::test]
async fn queue_enqueue_many_then_drain() {
    let (_container, url) = start_nats().await;
    let config = test_config(url);
    let provider = NatsQueueProvider::new(config).await.unwrap();
    provider.start().await.unwrap();

    for i in 0..20u32 {
        provider.queue_work(&format!("item-{i}"), QueueType::Workflow).await.unwrap();
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    for i in 0..20u32 {
        let got = provider.dequeue_work(QueueType::Workflow).await.unwrap();
        assert_eq!(got.as_deref(), Some(format!("item-{i}").as_str()));
    }
    assert!(provider.dequeue_work(QueueType::Workflow).await.unwrap().is_none());
}

#[tokio::test]
async fn lock_acquire_and_release() {
    let (_container, url) = start_nats().await;
    let config = test_config(url);
    let provider = NatsLockProvider::new(config).await.unwrap();
    provider.start().await.unwrap();

    assert!(provider.acquire_lock("resource-1").await.unwrap());
    assert!(!provider.acquire_lock("resource-1").await.unwrap());
    provider.release_lock("resource-1").await.unwrap();
    assert!(provider.acquire_lock("resource-1").await.unwrap());
}

#[tokio::test]
async fn lock_different_resources_are_independent() {
    let (_container, url) = start_nats().await;
    let config = test_config(url);
    let provider = NatsLockProvider::new(config).await.unwrap();
    provider.start().await.unwrap();

    assert!(provider.acquire_lock("resource-a").await.unwrap());
    assert!(provider.acquire_lock("resource-b").await.unwrap());
    assert!(!provider.acquire_lock("resource-a").await.unwrap());
    provider.release_lock("resource-a").await.unwrap();
    provider.release_lock("resource-b").await.unwrap();
}

#[tokio::test]
async fn lifecycle_publish_and_subscribe_locally() {
    let (_container, url) = start_nats().await;
    let config = test_config(url);
    let publisher = NatsLifecyclePublisher::new(config).await.unwrap();

    let mut subscriber = publisher.subscribe().unwrap();

    let event = LifecycleEvent::new("wf-1", "def-1", 1, LifecycleEventType::Started);
    publisher.publish(event.clone()).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv())
        .await
        .expect("should receive event")
        .expect("broadcast channel should not be closed");

    assert_eq!(received.workflow_instance_id, "wf-1");
    assert_eq!(received.event_type, LifecycleEventType::Started);
}

#[tokio::test]
async fn lifecycle_clustered_publish_subscribe() {
    let (_container, url) = start_nats().await;
    let config1 = test_config(url.clone());
    let config2 = NatsConfig {
        url,
        prefix: config1.prefix.clone(),
        ..config1.clone()
    };

    let publisher1 = Arc::new(NatsLifecyclePublisher::new(config1).await.unwrap());
    let publisher2 = Arc::new(NatsLifecyclePublisher::new(config2).await.unwrap());

    // Subscribe on publisher2, publish on publisher1.
    let mut subscriber = publisher2.subscribe().unwrap();

    // Give the remote subscriber time to attach before publishing.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let event = LifecycleEvent::new("wf-cluster", "def-1", 1, LifecycleEventType::Completed);
    publisher1.publish(event).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv())
        .await
        .expect("should receive clustered event")
        .expect("broadcast channel should not be closed");

    assert_eq!(received.workflow_instance_id, "wf-cluster");
    assert_eq!(received.event_type, LifecycleEventType::Completed);
}
