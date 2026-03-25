use wfe_core::queue_suite;

async fn make_provider() -> wfe_valkey::ValkeyQueueProvider {
    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    wfe_valkey::ValkeyQueueProvider::new("redis://localhost:6379", &prefix).await.unwrap()
}

queue_suite!(make_provider);
