mod common;

use wfe_core::lock_suite;

async fn make_provider() -> wfe_valkey::ValkeyLockProvider {
    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    let url = common::valkey_url().await;
    wfe_valkey::ValkeyLockProvider::new(&url, &prefix)
        .await
        .unwrap()
}

lock_suite!(make_provider);
