mod common;

use wfe_core::queue_suite;

async fn make_provider() -> wfe_valkey::ValkeyQueueProvider {
    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    let url = common::valkey_url().await;
    wfe_valkey::ValkeyQueueProvider::new(&url, &prefix)
        .await
        .unwrap()
}

queue_suite!(make_provider);
