//! Shared authentication helpers for commands.
//!
//! Resolves a bearer token from (in order):
//!   1. `--token` CLI flag
//!   2. `WFECTL_TOKEN` env var
//!   3. cached OIDC token at `~/.sunbeam/auth/{domain}.json` (refreshed if needed)

use anyhow::Result;

use crate::auth;

/// Resolve a bearer token to use for gRPC requests.
pub async fn resolve_token(cli_token: Option<&str>, issuer: &str) -> Result<String> {
    if let Some(token) = cli_token {
        if !token.is_empty() {
            return Ok(token.to_string());
        }
    }
    if let Ok(token) = std::env::var("WFECTL_TOKEN") {
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let domain = auth::domain_from_issuer(issuer)?;
    let stored = auth::ensure_valid(&domain).await?;
    Ok(stored.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize env-var tests to avoid races with parallel execution.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn resolve_uses_cli_token_first() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Save and clear env var so this test is independent.
        let saved = std::env::var("WFECTL_TOKEN").ok();
        unsafe { std::env::remove_var("WFECTL_TOKEN") };
        let token = resolve_token(Some("explicit-token"), "https://auth.example.com/")
            .await
            .unwrap();
        assert_eq!(token, "explicit-token");
        if let Some(v) = saved {
            unsafe { std::env::set_var("WFECTL_TOKEN", v) };
        }
    }

    #[tokio::test]
    async fn resolve_uses_env_when_no_cli() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::set_var("WFECTL_TOKEN", "env-token") };
        let token = resolve_token(None, "https://auth.example.com/")
            .await
            .unwrap();
        assert_eq!(token, "env-token");
        unsafe { std::env::remove_var("WFECTL_TOKEN") };
    }

    #[tokio::test]
    async fn resolve_skips_empty_cli_token() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        unsafe { std::env::set_var("WFECTL_TOKEN", "env-token") };
        let token = resolve_token(Some(""), "https://auth.example.com/")
            .await
            .unwrap();
        assert_eq!(token, "env-token");
        unsafe { std::env::remove_var("WFECTL_TOKEN") };
    }
}
