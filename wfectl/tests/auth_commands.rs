//! Tests for `whoami` and `logout` commands using a temp HOME with a fake token.

use chrono::Utc;
use std::path::PathBuf;
use std::sync::Mutex;

use wfectl::auth::{StoredToken, save_token};
use wfectl::commands::{logout, whoami};
use wfectl::config::Config;
use wfectl::output::OutputFormat;

/// Serialize HOME-mutating tests so they don't race.
static HOME_LOCK: Mutex<()> = Mutex::new(());

fn make_test_token(domain: &str) -> StoredToken {
    StoredToken {
        access_token: "ory_at_test".into(),
        refresh_token: Some("ory_rt_test".into()),
        // {"email":"alice@test.com","name":"Alice","groups":["admin","employee"]}
        id_token: Some("h.eyJlbWFpbCI6ImFsaWNlQHRlc3QuY29tIiwibmFtZSI6IkFsaWNlIiwiZ3JvdXBzIjpbImFkbWluIiwiZW1wbG95ZWUiXX0.s".into()),
        expires_at: Utc::now() + chrono::Duration::seconds(3600),
        issuer: "https://auth.test.com/".into(),
        domain: domain.into(),
    }
}

fn with_temp_home<F: FnOnce(PathBuf)>(f: F) {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };
    f(tmp.path().to_path_buf());
    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn whoami_table_format_with_full_claims() {
    with_temp_home(|_| {});
    // Re-acquire the lock for the async section.
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let token = make_test_token("test.com");
    save_token(&token).unwrap();

    let cfg = Config {
        server: "http://localhost".into(),
        issuer: "https://auth.test.com/".into(),
        default_format: Default::default(),
    };
    let args = whoami::WhoamiArgs { issuer: None };
    whoami::run(args, &cfg, OutputFormat::Table).await.unwrap();

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn whoami_json_format() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let token = make_test_token("test.com");
    save_token(&token).unwrap();

    let cfg = Config {
        server: "http://localhost".into(),
        issuer: "https://auth.test.com/".into(),
        default_format: Default::default(),
    };
    let args = whoami::WhoamiArgs { issuer: None };
    whoami::run(args, &cfg, OutputFormat::Json).await.unwrap();

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn whoami_when_not_logged_in() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let cfg = Config {
        server: "http://localhost".into(),
        issuer: "https://auth.notlogged.in/".into(),
        default_format: Default::default(),
    };
    let args = whoami::WhoamiArgs { issuer: None };
    // Should not panic, just print a message.
    whoami::run(args, &cfg, OutputFormat::Table).await.unwrap();

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn whoami_with_explicit_issuer_arg() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let token = make_test_token("override.com");
    save_token(&token).unwrap();

    let cfg = Config::default();
    let args = whoami::WhoamiArgs {
        issuer: Some("https://auth.override.com/".into()),
    };
    whoami::run(args, &cfg, OutputFormat::Table).await.unwrap();

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn logout_removes_token() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let token = make_test_token("test.com");
    save_token(&token).unwrap();

    let cfg = Config {
        server: "http://localhost".into(),
        issuer: "https://auth.test.com/".into(),
        default_format: Default::default(),
    };
    let args = logout::LogoutArgs { issuer: None };
    logout::run(args, &cfg).await.unwrap();

    // Verify the token file is gone.
    let path = wfectl::auth::token_path("test.com");
    assert!(!path.exists());

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn logout_when_not_logged_in_is_noop() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let cfg = Config {
        server: "http://localhost".into(),
        issuer: "https://auth.nothing.com/".into(),
        default_format: Default::default(),
    };
    let args = logout::LogoutArgs { issuer: None };
    logout::run(args, &cfg).await.unwrap();

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}

#[tokio::test]
async fn logout_with_explicit_issuer() {
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let saved_home = std::env::var("HOME").ok();
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let token = make_test_token("explicit.com");
    save_token(&token).unwrap();

    let cfg = Config::default();
    let args = logout::LogoutArgs {
        issuer: Some("https://auth.explicit.com/".into()),
    };
    logout::run(args, &cfg).await.unwrap();

    if let Some(h) = saved_home {
        unsafe { std::env::set_var("HOME", h) };
    }
}
