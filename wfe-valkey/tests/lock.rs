use wfe_core::lock_suite;

async fn make_provider() -> wfe_valkey::ValkeyLockProvider {
    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    wfe_valkey::ValkeyLockProvider::new("redis://localhost:6379", &prefix).await.unwrap()
}

lock_suite!(make_provider);
