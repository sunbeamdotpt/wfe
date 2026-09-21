mod common;

use wfe_core::persistence_suite;
use wfe_core::traits::PersistenceProvider;

async fn make_provider() -> wfe_postgres::PostgresPersistenceProvider {
    let url = common::database_url().await;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .expect("Failed to connect to PostgreSQL testcontainer");

    let provider = wfe_postgres::PostgresPersistenceProvider::from_pool(pool);
    provider.ensure_store_exists().await.unwrap();
    provider.truncate_all().await.unwrap();
    provider
}

persistence_suite!(make_provider);

/// The sqlx migration tracking table must live inside the `wfc` schema, not
/// `public`, so a host application running its own sqlx migrations against
/// this database keeps an untouched `_sqlx_migrations` of its own.
#[tokio::test]
async fn migration_tracking_lands_inside_wfc_schema() {
    let _ = make_provider().await;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&common::database_url().await)
        .await
        .expect("Failed to connect to PostgreSQL testcontainer");

    let in_wfc: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'wfc' AND table_name = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(in_wfc.0, 1, "_sqlx_migrations should live in wfc");

    let in_public: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(in_public.0, 0, "_sqlx_migrations must not leak into public");
}
