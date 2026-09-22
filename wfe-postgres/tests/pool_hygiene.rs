//! The migration connection must return to the pool exactly as it was found:
//! a lingering `SET search_path` would redirect later borrowers' unqualified
//! queries into WFE's schema — real sabotage when the pool belongs to a host
//! application (`from_pool`).
//!
//! Runs against a `WFE_PG_TEST_URL` override or a Postgres testcontainer.
mod common;

use wfe_core::traits::PersistenceProvider;

async fn show_search_path(pool: &sqlx::PgPool) -> String {
    sqlx::query_scalar("SHOW search_path")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_does_not_poison_the_shared_connection() {
    // One connection: the post-migration checkout is provably the same
    // physical connection the migration ran on.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&common::database_url().await)
        .await
        .unwrap();

    let before = show_search_path(&pool).await;

    let provider = wfe_postgres::PostgresPersistenceProvider::from_pool(pool.clone());
    provider.ensure_store_exists().await.unwrap();

    let after = show_search_path(&pool).await;
    assert_eq!(
        before, after,
        "search_path must be restored on the pooled connection after migrating"
    );

    // And WFE's tables are not reachable unqualified from the shared pool.
    let unqualified = sqlx::query("SELECT COUNT(*) FROM workflows")
        .fetch_optional(&pool)
        .await;
    assert!(
        unqualified.is_err(),
        "workflows must not resolve without schema qualification"
    );
}

#[tokio::test]
async fn host_application_search_path_survives_migration() {
    // The shared-pool scenario: the application customizes search_path per
    // connection in after_connect, then hands the pool to wfe via from_pool.
    // The customization must still hold after wfe migrates.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::Executor::execute(
                    conn,
                    "CREATE SCHEMA IF NOT EXISTS app; SET search_path TO app",
                )
                .await?;
                Ok(())
            })
        })
        .connect(&common::database_url().await)
        .await
        .unwrap();

    let provider = wfe_postgres::PostgresPersistenceProvider::from_pool_with(
        pool.clone(),
        wfe_postgres::PostgresOptions::default(),
    )
    .unwrap();
    provider.ensure_store_exists().await.unwrap();

    let after = show_search_path(&pool).await;
    assert_eq!(
        after, "app",
        "the application's after_connect search_path must survive wfe's migration"
    );
}
