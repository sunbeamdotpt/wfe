use wfe_core::persistence_suite;

async fn make_provider() -> wfe_sqlite::SqlitePersistenceProvider {
    wfe_sqlite::SqlitePersistenceProvider::new(":memory:")
        .await
        .unwrap()
}

persistence_suite!(|| async { make_provider().await });
