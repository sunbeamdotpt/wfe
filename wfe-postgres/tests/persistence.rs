use wfe_core::persistence_suite;
use wfe_core::traits::PersistenceProvider;

/// Override with WFE_PG_TEST_URL when localhost:5432 is occupied by another
/// PostgreSQL (e.g. a native install shadowing a Docker-published one).
fn database_url() -> String {
    std::env::var("WFE_PG_TEST_URL")
        .unwrap_or_else(|_| "postgres://wfe:wfe@localhost:5432/wfe_test".to_string())
}

async fn make_provider() -> wfe_postgres::PostgresPersistenceProvider {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url())
        .await
        .expect("Failed to connect to PostgreSQL. Is the database running?");

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
        .connect(&database_url())
        .await
        .expect("Failed to connect to PostgreSQL. Is the database running?");

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
