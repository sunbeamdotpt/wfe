//! OAuth2 Authorization Code flow with PKCE for the wfectl CLI.
//!
//! Reuses the `sunbeam-cli` Ory Hydra client and stores tokens at the same
//! path as the sunbeam CLI (`~/.sunbeam/auth/{domain}.json`) so a single
//! login works for both tools.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Shared OIDC client used by both sunbeam and wfectl CLIs.
pub const CLIENT_ID: &str = "sunbeam-cli";
/// Standard OIDC scopes.
pub const SCOPES: &str = "openid email profile offline_access";
/// Loopback callback ports tried in order.
pub const CALLBACK_PORTS: [u16; 5] = [9876, 9877, 9878, 9879, 9880];

/// Persisted OAuth token state, written to `~/.sunbeam/auth/{domain}.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub issuer: String,
    pub domain: String,
}

impl StoredToken {
    /// True if the access token has at least `min_remaining` left.
    pub fn is_valid_for(&self, min_remaining: Duration) -> bool {
        let now = Utc::now();
        let cutoff = now + chrono::Duration::from_std(min_remaining).unwrap_or_default();
        self.expires_at > cutoff
    }

    /// Decode and return claims from the embedded id_token (no signature
    /// verification -- relies on TLS).
    pub fn id_claims(&self) -> Option<serde_json::Value> {
        let token = self.id_token.as_deref()?;
        let mut parts = token.split('.');
        let _header = parts.next()?;
        let payload = parts.next()?;
        let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

/// PKCE verifier + challenge pair.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// Generate a fresh PKCE pair (32-byte verifier, S256 challenge).
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = challenge_for(&verifier);
        Self {
            verifier,
            challenge,
        }
    }
}

/// Compute the S256 PKCE challenge for a given verifier.
pub fn challenge_for(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

/// Generate a CSRF state token.
pub fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Where the token cache file lives for a given OIDC domain.
pub fn token_path(domain: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".sunbeam/auth").join(format!("{domain}.json"))
}

/// Read the cached token for a domain. Returns Ok(None) if not present.
pub fn load_token(domain: &str) -> Result<Option<StoredToken>> {
    let path = token_path(domain);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("failed to read token cache at {}", path.display()))?;
    let token: StoredToken = serde_json::from_slice(&bytes)
        .with_context(|| format!("failed to parse token cache at {}", path.display()))?;
    Ok(Some(token))
}

/// Persist a token to the cache, creating parent directories as needed.
/// File is written with mode 0600 on Unix.
pub fn save_token(token: &StoredToken) -> Result<()> {
    let path = token_path(&token.domain);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create token dir {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(token).context("failed to serialize token")?;
    std::fs::write(&path, &bytes)
        .with_context(|| format!("failed to write token cache to {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).ok();
    }
    Ok(())
}

/// Delete the token cache for a domain. Returns true if a file was deleted.
pub fn delete_token(domain: &str) -> Result<bool> {
    let path = token_path(domain);
    if !path.exists() {
        return Ok(false);
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("failed to delete token cache at {}", path.display()))?;
    Ok(true)
}

/// Extract the domain from an OIDC issuer URL.
///
/// `https://auth.sunbeam.pt/` -> `sunbeam.pt`
/// `https://auth.example.com:8443/realms/foo` -> `example.com`
pub fn domain_from_issuer(issuer: &str) -> Result<String> {
    let url = url::Url::parse(issuer).with_context(|| format!("invalid issuer URL: {issuer}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("issuer has no host"))?;
    // Strip a leading "auth." subdomain so all sunbeam.pt CLIs share a token cache.
    let domain = host.strip_prefix("auth.").unwrap_or(host).to_string();
    Ok(domain)
}

/// OIDC discovery document subset.
#[derive(Debug, Clone, Deserialize)]
pub struct DiscoveryDoc {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
}

/// Fetch the OIDC discovery document for an issuer.
pub async fn discover(issuer: &str) -> Result<DiscoveryDoc> {
    let trimmed = issuer.trim_end_matches('/');
    let url = format!("{trimmed}/.well-known/openid-configuration");
    let resp = reqwest::get(&url)
        .await
        .with_context(|| format!("failed to fetch OIDC discovery from {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "OIDC discovery returned HTTP {} from {}",
            resp.status(),
            url
        ));
    }
    let doc: DiscoveryDoc = resp
        .json()
        .await
        .context("failed to parse discovery JSON")?;
    Ok(doc)
}

/// Token endpoint response.
#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl TokenResponse {
    fn into_stored(self, issuer: &str, domain: &str) -> StoredToken {
        let expires_in = self.expires_in.unwrap_or(3600);
        StoredToken {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            id_token: self.id_token,
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in),
            issuer: issuer.to_string(),
            domain: domain.to_string(),
        }
    }
}

/// Exchange an authorization code for tokens.
pub async fn exchange_code(
    discovery: &DiscoveryDoc,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    issuer: &str,
    domain: &str,
) -> Result<StoredToken> {
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ];
    let resp = reqwest::Client::new()
        .post(&discovery.token_endpoint)
        .form(&params)
        .send()
        .await
        .context("token endpoint request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("token endpoint returned HTTP {status}: {body}"));
    }
    let token: TokenResponse = resp
        .json()
        .await
        .context("failed to parse token response")?;
    Ok(token.into_stored(issuer, domain))
}

/// Use a refresh token to obtain a fresh access token.
pub async fn refresh(token: &StoredToken) -> Result<StoredToken> {
    let refresh_token = token
        .refresh_token
        .as_deref()
        .ok_or_else(|| anyhow!("no refresh token available"))?;

    let discovery = discover(&token.issuer).await?;

    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];
    let resp = reqwest::Client::new()
        .post(&discovery.token_endpoint)
        .form(&params)
        .send()
        .await
        .context("refresh request failed")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("refresh endpoint returned HTTP {status}: {body}"));
    }
    let resp: TokenResponse = resp
        .json()
        .await
        .context("failed to parse refresh response")?;
    let mut new_token = resp.into_stored(&token.issuer, &token.domain);
    // Some IdPs don't return a new refresh token; preserve the original.
    if new_token.refresh_token.is_none() {
        new_token.refresh_token = token.refresh_token.clone();
    }
    Ok(new_token)
}

/// Load a token, refreshing it if it has less than 60s remaining.
pub async fn ensure_valid(domain: &str) -> Result<StoredToken> {
    let token =
        load_token(domain)?.ok_or_else(|| anyhow!("not logged in -- run `wfectl login` first"))?;
    if token.is_valid_for(Duration::from_secs(60)) {
        return Ok(token);
    }
    let refreshed = refresh(&token).await.context("token refresh failed")?;
    save_token(&refreshed)?;
    Ok(refreshed)
}

/// Build the authorization URL for the browser.
pub fn build_auth_url(
    discovery: &DiscoveryDoc,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let mut url = url::Url::parse(&discovery.authorization_endpoint)
        .expect("authorization_endpoint must be a valid URL");
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state);
    url.to_string()
}

/// Open the user's browser at a given URL (best effort).
pub fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "linux")]
    let prog = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let prog: &str = "";

    if !prog.is_empty() {
        let _ = std::process::Command::new(prog).arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn pkce_challenge_is_deterministic() {
        let c1 = challenge_for("verifier");
        let c2 = challenge_for("verifier");
        assert_eq!(c1, c2);
    }

    #[test]
    fn pkce_challenge_known_value() {
        // RFC 7636 example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = challenge_for(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn pkce_generate_produces_unique_pairs() {
        let a = Pkce::generate();
        let b = Pkce::generate();
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.challenge, b.challenge);
        // Verifier round-trips through challenge.
        assert_eq!(a.challenge, challenge_for(&a.verifier));
    }

    #[test]
    fn random_state_is_unique_and_long() {
        let s1 = random_state();
        let s2 = random_state();
        assert_ne!(s1, s2);
        assert!(s1.len() >= 22);
    }

    #[test]
    fn domain_from_issuer_strips_auth_subdomain() {
        assert_eq!(
            domain_from_issuer("https://auth.sunbeam.pt/").unwrap(),
            "sunbeam.pt"
        );
        assert_eq!(
            domain_from_issuer("https://auth.example.com").unwrap(),
            "example.com"
        );
        assert_eq!(
            domain_from_issuer("https://example.com/").unwrap(),
            "example.com"
        );
    }

    #[test]
    fn domain_from_issuer_invalid_url() {
        assert!(domain_from_issuer("not a url").is_err());
    }

    #[test]
    fn token_path_is_under_sunbeam_auth() {
        let p = token_path("sunbeam.pt");
        assert!(
            p.to_string_lossy()
                .ends_with(".sunbeam/auth/sunbeam.pt.json")
        );
    }

    #[test]
    fn stored_token_serde_round_trip() {
        let token = StoredToken {
            access_token: "ory_at_abc".into(),
            refresh_token: Some("ory_rt_xyz".into()),
            id_token: Some("eyJhbGc".into()),
            expires_at: "2030-01-01T00:00:00Z".parse().unwrap(),
            issuer: "https://auth.sunbeam.pt/".into(),
            domain: "sunbeam.pt".into(),
        };
        let json = serde_json::to_string(&token).unwrap();
        let parsed: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.access_token, "ory_at_abc");
        assert_eq!(parsed.refresh_token, Some("ory_rt_xyz".into()));
        assert_eq!(parsed.domain, "sunbeam.pt");
    }

    #[test]
    fn token_validity_check() {
        let valid = StoredToken {
            access_token: "x".into(),
            refresh_token: None,
            id_token: None,
            expires_at: Utc::now() + chrono::Duration::seconds(3600),
            issuer: "x".into(),
            domain: "x".into(),
        };
        assert!(valid.is_valid_for(Duration::from_secs(60)));

        let expiring = StoredToken {
            access_token: "x".into(),
            refresh_token: None,
            id_token: None,
            expires_at: Utc::now() + chrono::Duration::seconds(30),
            issuer: "x".into(),
            domain: "x".into(),
        };
        assert!(!expiring.is_valid_for(Duration::from_secs(60)));

        let expired = StoredToken {
            access_token: "x".into(),
            refresh_token: None,
            id_token: None,
            expires_at: Utc::now() - chrono::Duration::seconds(10),
            issuer: "x".into(),
            domain: "x".into(),
        };
        assert!(!expired.is_valid_for(Duration::from_secs(0)));
    }

    #[test]
    fn id_claims_decodes_jwt_payload() {
        // {"email":"user@example.com","name":"Test"}
        let payload = "eyJlbWFpbCI6InVzZXJAZXhhbXBsZS5jb20iLCJuYW1lIjoiVGVzdCJ9";
        let token = StoredToken {
            access_token: "x".into(),
            refresh_token: None,
            id_token: Some(format!("h.{payload}.s")),
            expires_at: Utc::now(),
            issuer: "x".into(),
            domain: "x".into(),
        };
        let claims = token.id_claims().unwrap();
        assert_eq!(claims["email"], "user@example.com");
        assert_eq!(claims["name"], "Test");
    }

    #[test]
    fn id_claims_returns_none_without_token() {
        let token = StoredToken {
            access_token: "x".into(),
            refresh_token: None,
            id_token: None,
            expires_at: Utc::now(),
            issuer: "x".into(),
            domain: "x".into(),
        };
        assert!(token.id_claims().is_none());
    }

    #[test]
    fn build_auth_url_contains_pkce_and_state() {
        let discovery = DiscoveryDoc {
            authorization_endpoint: "https://auth.example.com/oauth2/auth".into(),
            token_endpoint: "https://auth.example.com/oauth2/token".into(),
        };
        let url = build_auth_url(
            &discovery,
            "http://127.0.0.1:9876/callback",
            "state-123",
            "challenge-abc",
        );
        assert!(url.contains("client_id=sunbeam-cli"));
        assert!(url.contains("code_challenge=challenge-abc"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=state-123"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("scope=openid"));
    }

    #[test]
    fn save_load_delete_token_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        // Override HOME so token_path resolves into the temp dir.
        unsafe { std::env::set_var("HOME", tmp.path()) };

        let token = StoredToken {
            access_token: "ory_at_test".into(),
            refresh_token: Some("ory_rt_test".into()),
            id_token: None,
            expires_at: Utc::now() + chrono::Duration::seconds(3600),
            issuer: "https://auth.test.com/".into(),
            domain: "test.com".into(),
        };

        save_token(&token).unwrap();
        let loaded = load_token("test.com").unwrap().unwrap();
        assert_eq!(loaded.access_token, "ory_at_test");

        let deleted = delete_token("test.com").unwrap();
        assert!(deleted);
        let after = load_token("test.com").unwrap();
        assert!(after.is_none());

        // Deleting a non-existent token returns false, not error.
        let again = delete_token("test.com").unwrap();
        assert!(!again);
    }
}
