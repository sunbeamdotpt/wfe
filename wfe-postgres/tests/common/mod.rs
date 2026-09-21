//! Shared Postgres endpoint for the integration suites: a `WFE_PG_TEST_URL`
//! override when set, otherwise a Postgres testcontainer on whatever Docker
//! endpoint the environment resolves (the workspace's remote TLS daemon
//! included). One container per test binary; `ensure_store_exists` +
//! `truncate_all` reset state between tests.

use std::sync::Mutex;

use testcontainers::{ContainerAsync, GenericImage};

static URL: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
static CONTAINERS: Mutex<Vec<ContainerAsync<GenericImage>>> = Mutex::new(Vec::new());

pub async fn database_url() -> String {
    if let Ok(url) = std::env::var("WFE_PG_TEST_URL") {
        return url;
    }
    URL.get_or_init(|| async {
        let container = wfe_core::test_support::containers::Postgres::new()
            .start()
            .await
            .expect("failed to start Postgres testcontainer");
        let url = wfe_core::test_support::containers::Postgres::url(&container)
            .await
            .expect("failed to resolve Postgres URL");
        CONTAINERS.lock().unwrap().push(container);
        url
    })
    .await
    .clone()
}
