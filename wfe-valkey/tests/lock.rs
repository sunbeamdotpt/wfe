use wfe_core::lock_suite;

/// Override with WFE_VALKEY_TEST_URL when localhost:6379 is occupied or the
/// Valkey server runs elsewhere (e.g. published on a remote Docker daemon).
fn valkey_url() -> String {
    std::env::var("WFE_VALKEY_TEST_URL")
        .unwrap_or_else(|_| "redis://localhost:6379".to_string())
}

async fn make_provider() -> wfe_valkey::ValkeyLockProvider {
    let prefix = format!("wfe_test_{}", uuid::Uuid::new_v4().simple());
    wfe_valkey::ValkeyLockProvider::new(&valkey_url(), &prefix)
        .await
        .unwrap()
}

lock_suite!(make_provider);
