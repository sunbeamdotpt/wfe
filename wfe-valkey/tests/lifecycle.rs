use std::time::Duration;

use wfe_core::models::{LifecycleEvent, LifecycleEventType};
use wfe_core::traits::LifecyclePublisher;

fn redis_available() -> bool {
    redis::Client::open("redis://localhost:6379")
        .and_then(|c| c.get_connection())
        .is_ok()
}

#[tokio::test]
async fn publish_subscribe_round_trip() {
    if !redis_available() {
        eprintln!("Skipping test: Valkey/Redis not available");
        return;
    }

    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    let publisher = wfe_valkey::ValkeyLifecyclePublisher::new("redis://localhost:6379", &prefix)
        .await
        .unwrap();

    let instance_id = "wf-lifecycle-test-1";
    let channel = format!("{}:lifecycle:{}", prefix, instance_id);

    // Set up a subscriber in a background task.
    let sub_client = redis::Client::open("redis://localhost:6379").unwrap();
    let mut pubsub = sub_client.get_async_pubsub().await.unwrap();
    pubsub.subscribe(&channel).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);

    let handle = tokio::spawn(async move {
        let mut stream = pubsub.into_on_message();
        use tokio_stream::StreamExt;
        if let Some(msg) = stream.next().await {
            let payload: String = msg.get_payload().unwrap();
            let _ = tx.send(payload).await;
        }
    });

    // Small delay to ensure the subscription is active before publishing.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let event = LifecycleEvent::new(instance_id, "def-1", 1, LifecycleEventType::Started);
    publisher.publish(event).await.unwrap();

    // Wait for the message with a timeout.
    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timed out waiting for lifecycle event")
        .expect("Channel closed without receiving a message");

    let deserialized: LifecycleEvent = serde_json::from_str(&received).unwrap();
    assert_eq!(deserialized.workflow_instance_id, instance_id);
    assert_eq!(deserialized.event_type, LifecycleEventType::Started);

    handle.abort();
}

#[tokio::test]
async fn publish_to_all_channel() {
    if !redis_available() {
        eprintln!("Skipping test: Valkey/Redis not available");
        return;
    }

    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    let publisher = wfe_valkey::ValkeyLifecyclePublisher::new("redis://localhost:6379", &prefix)
        .await
        .unwrap();

    let all_channel = format!("{}:lifecycle:all", prefix);

    let sub_client = redis::Client::open("redis://localhost:6379").unwrap();
    let mut pubsub = sub_client.get_async_pubsub().await.unwrap();
    pubsub.subscribe(&all_channel).await.unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(1);

    let handle = tokio::spawn(async move {
        let mut stream = pubsub.into_on_message();
        use tokio_stream::StreamExt;
        if let Some(msg) = stream.next().await {
            let payload: String = msg.get_payload().unwrap();
            let _ = tx.send(payload).await;
        }
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    let event = LifecycleEvent::new("wf-all-test", "def-1", 1, LifecycleEventType::Completed);
    publisher.publish(event).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("Timed out waiting for lifecycle event on 'all' channel")
        .expect("Channel closed");

    let deserialized: LifecycleEvent = serde_json::from_str(&received).unwrap();
    assert_eq!(deserialized.workflow_instance_id, "wf-all-test");
    assert_eq!(deserialized.event_type, LifecycleEventType::Completed);

    handle.abort();
}
