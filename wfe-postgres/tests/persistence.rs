use wfe_core::persistence_suite;
use wfe_core::traits::PersistenceProvider;

async fn make_provider() -> wfe_postgres::PostgresPersistenceProvider {
    let database_url = "postgres://wfe:wfe@localhost:5433/wfe_test";

    let provider = wfe_postgres::PostgresPersistenceProvider::new(database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Is the database running?");

    provider.ensure_store_exists().await.unwrap();
    provider.truncate_all().await.unwrap();
    provider
}

persistence_suite!(make_provider);
