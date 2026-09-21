//! Shared Valkey endpoint for the integration suites: a `WFE_VALKEY_TEST_URL`
//! override when set, otherwise a Valkey testcontainer on whatever Docker
//! endpoint the environment resolves (the workspace's remote TLS daemon
//! included). One container per test binary; unique key prefixes keep tests
//! independent when several binaries share a server.

use std::sync::Mutex;

use testcontainers::{ContainerAsync, GenericImage};

static URL: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();
static CONTAINERS: Mutex<Vec<ContainerAsync<GenericImage>>> = Mutex::new(Vec::new());

pub async fn valkey_url() -> String {
    if let Ok(url) = std::env::var("WFE_VALKEY_TEST_URL") {
        return url;
    }
    URL.get_or_init(|| async {
        let container = wfe_core::test_support::containers::Valkey::new()
            .start()
            .await
            .expect("failed to start Valkey testcontainer");
        let url = wfe_core::test_support::containers::Valkey::url(&container)
            .await
            .expect("failed to resolve Valkey URL");
        CONTAINERS.lock().unwrap().push(container);
        url
    })
    .await
    .clone()
}
