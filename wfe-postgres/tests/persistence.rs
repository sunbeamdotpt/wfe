use wfe_core::persistence_suite;
use wfe_core::traits::PersistenceProvider;

async fn make_provider() -> wfe_postgres::PostgresPersistenceProvider {
    let database_url = "postgres://wfe:wfe@localhost:5432/wfe_test";

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(database_url)
        .await
        .expect("Failed to connect to PostgreSQL. Is the database running?");

    let provider = wfe_postgres::PostgresPersistenceProvider::from_pool(pool);
    provider.ensure_store_exists().await.unwrap();
    provider.truncate_all().await.unwrap();
    provider
}

persistence_suite!(make_provider);
