//! Tests for OAuth2/OIDC code paths in auth.rs that talk to a real (mock) HTTP server.

use chrono::Utc;
use std::sync::Mutex;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use wfectl::auth::{StoredToken, discover, ensure_valid, exchange_code, refresh, save_token};

static HOME_LOCK: Mutex<()> = Mutex::new(());

fn discovery_body(server: &MockServer) -> serde_json::Value {
    serde_json::json!({
        "issuer": server.uri(),
        "authorization_endpoint": format!("{}/oauth2/auth", server.uri()),
        "token_endpoint": format!("{}/oauth2/token", server.uri()),
    })
}

#[tokio::test]
async fn discover_fetches_endpoints() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(discovery_body(&server)))
        .mount(&server)
        .await;

    let doc = discover(&server.uri()).await.unwrap();
    assert!(doc.authorization_endpoint.ends_with("/oauth2/auth"));
    assert!(doc.token_endpoint.ends_with("/oauth2/token"));
}

#[tokio::test]
async fn discover_handles_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let result = discover(&server.uri()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn discover_handles_invalid_json() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;
    let result = discover(&server.uri()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn exchange_code_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ory_at_new",
            "refresh_token": "ory_rt_new",
            "id_token": "header.eyJzdWIiOiJ1c2VyIn0.sig",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let discovery = wfectl::auth::DiscoveryDoc {
        authorization_endpoint: format!("{}/oauth2/auth", server.uri()),
        token_endpoint: format!("{}/oauth2/token", server.uri()),
    };
    let token = exchange_code(
        &discovery,
        "auth-code",
        "verifier",
        "http://127.0.0.1:9876/callback",
        &server.uri(),
        "test.com",
    )
    .await
    .unwrap();
    assert_eq!(token.access_token, "ory_at_new");
    assert_eq!(token.refresh_token, Some("ory_rt_new".into()));
}

#[tokio::test]
async fn exchange_code_handles_error_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": "invalid_grant",
        })))
        .mount(&server)
        .await;

    let discovery = wfectl::auth::DiscoveryDoc {
        authorization_endpoint: format!("{}/oauth2/auth", server.uri()),
        token_endpoint: format!("{}/oauth2/token", server.uri()),
    };
    let result = exchange_code(&discovery, "bad", "v", "uri", &server.uri(), "test.com").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn refresh_token_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(discovery_body(&server)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ory_at_refreshed",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let token = StoredToken {
        access_token: "ory_at_old".into(),
        refresh_token: Some("ory_rt_old".into()),
        id_token: None,
        expires_at: Utc::now() - chrono::Duration::seconds(10),
        issuer: server.uri(),
        domain: "test.com".into(),
    };
    let refreshed = refresh(&token).await.unwrap();
    assert_eq!(refreshed.access_token, "ory_at_refreshed");
    // Old refresh token preserved when new one isn't returned.
    assert_eq!(refreshed.refresh_token, Some("ory_rt_old".into()));
}

#[tokio::test]
async fn refresh_without_refresh_token_fails() {
    let token = StoredToken {
        access_token: "ory_at".into(),
        refresh_token: None,
        id_token: None,
        expires_at: Utc::now(),
        issuer: "https://auth.test.com/".into(),
        domain: "test.com".into(),
    };
    let result = refresh(&token).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn refresh_handles_token_endpoint_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(discovery_body(&server)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let token = StoredToken {
        access_token: "ory_at_old".into(),
        refresh_token: Some("ory_rt_old".into()),
        id_token: None,
        expires_at: Utc::now() - chrono::Duration::seconds(10),
        issuer: server.uri(),
        domain: "test.com".into(),
    };
    let result = refresh(&token).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn ensure_valid_returns_existing_when_fresh() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let token = StoredToken {
        access_token: "ory_at_fresh".into(),
        refresh_token: Some("ory_rt_x".into()),
        id_token: None,
        expires_at: Utc::now() + chrono::Duration::seconds(3600),
        issuer: "https://auth.fresh.com/".into(),
        domain: "fresh.com".into(),
    };
    save_token(&token).unwrap();

    let result = ensure_valid("fresh.com").await.unwrap();
    assert_eq!(result.access_token, "ory_at_fresh");

    if let Some(h) = saved {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn ensure_valid_refreshes_when_stale() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(discovery_body(&server)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ory_at_refreshed_via_ensure",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    let domain = "stale.com";
    let token = StoredToken {
        access_token: "ory_at_old".into(),
        refresh_token: Some("ory_rt_old".into()),
        id_token: None,
        expires_at: Utc::now() - chrono::Duration::seconds(10),
        issuer: server.uri(),
        domain: domain.into(),
    };
    save_token(&token).unwrap();

    let result = ensure_valid(domain).await.unwrap();
    assert_eq!(result.access_token, "ory_at_refreshed_via_ensure");

    if let Some(h) = saved {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn ensure_valid_errors_when_no_token() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let result = ensure_valid("nonexistent.com").await;
    assert!(result.is_err());

    if let Some(h) = saved {
        unsafe { std::env::set_var("HOME", h) };
    }
}
