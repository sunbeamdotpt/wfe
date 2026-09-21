//! Integration tests for storing WFE data alongside a host application's own
//! tables: a dedicated schema plus a table-name prefix, with migration
//! bookkeeping kept inside the same schema so it cannot collide with the
//! application's `_sqlx_migrations` in `public`.
//!
//! Runs against a `WFE_PG_TEST_URL` override or a Postgres testcontainer.
mod common;

use sqlx::Row;
use wfe_core::persistence_suite;
use wfe_core::traits::{PersistenceProvider, WorkflowRepository};

/// Distinct schema + prefix so the suite proves full isolation from the
/// default `wfc` layout exercised by `tests/persistence.rs`.
const TEST_SCHEMA: &str = "wfe_iso";
const TEST_PREFIX: &str = "wfe_";

async fn make_isolated_provider() -> wfe_postgres::PostgresPersistenceProvider {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&common::database_url().await)
        .await
        .expect("Failed to connect to PostgreSQL testcontainer");

    let provider = wfe_postgres::PostgresPersistenceProvider::from_pool_with(
        pool,
        wfe_postgres::PostgresOptions {
            schema: TEST_SCHEMA.to_string(),
            table_prefix: TEST_PREFIX.to_string(),
        },
    )
    .expect("options are valid");

    provider.ensure_store_exists().await.unwrap();
    provider.truncate_all().await.unwrap();
    provider
}

persistence_suite!(make_isolated_provider);

#[tokio::test]
async fn tables_land_in_custom_schema_with_prefix() {
    let _ = make_isolated_provider().await;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&common::database_url().await)
        .await
        .unwrap();

    for table in [
        "wfe_workflows",
        "wfe_definition_sequences",
        "wfe_execution_pointers",
        "wfe_events",
        "wfe_event_subscriptions",
        "wfe_execution_errors",
        "wfe_scheduled_commands",
    ] {
        let exists: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM information_schema.tables \
             WHERE table_schema = $1 AND table_name = $2",
        )
        .bind(TEST_SCHEMA)
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(exists.0, 1, "{TEST_SCHEMA}.{table} should exist");
    }

    // The prefixed tracking table is in the custom schema; nothing WFE-related
    // leaked into public, where a host application's tables live.
    let prefixed_tracking: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = $1 AND table_name = '_wfe_sqlx_migrations'",
    )
    .bind(TEST_SCHEMA)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(prefixed_tracking.0, 1);

    let public_wfe: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name LIKE '%workflows%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(public_wfe.0, 0, "no WFE tables in public");

    let unprefixed_in_custom: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM information_schema.tables \
         WHERE table_schema = $1 AND table_name = 'workflows'",
    )
    .bind(TEST_SCHEMA)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(unprefixed_in_custom.0, 0, "tables must carry the prefix");
}

#[tokio::test]
async fn prefixed_store_is_usable_end_to_end() {
    let provider = make_isolated_provider().await;

    let instance =
        wfe_core::models::WorkflowInstance::new("iso-wf", 1, serde_json::json!({"a": 1}));
    let id = provider.create_new_workflow(&instance).await.unwrap();

    let fetched = provider.get_workflow_instance(&id).await.unwrap();
    assert_eq!(fetched.data, serde_json::json!({"a": 1}));

    // Data is really in the prefixed table, not somewhere else.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&common::database_url().await)
        .await
        .unwrap();
    let row = sqlx::query(&format!(
        "SELECT data FROM {TEST_SCHEMA}.{TEST_PREFIX}workflows WHERE id = $1"
    ))
    .bind(&id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let data: serde_json::Value = row.get("data");
    assert_eq!(data, serde_json::json!({"a": 1}));
}

#[tokio::test]
async fn migration_tracking_is_idempotent_under_prefix() {
    let provider = make_isolated_provider().await;
    // A second run must recognize its own history rather than re-apply or
    // fail on a checksum mismatch.
    provider.ensure_store_exists().await.unwrap();
    provider.ensure_store_exists().await.unwrap();
}
