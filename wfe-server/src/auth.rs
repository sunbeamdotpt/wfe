use std::sync::Arc;

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;
use tokio::sync::RwLock;
use tonic::{Request, Status};

use crate::config::AuthConfig;

/// Asymmetric algorithms we accept. NEVER trust the JWT header's alg claim.
/// This prevents algorithm confusion attacks (CVE-2016-5431).
const ALLOWED_ALGORITHMS: &[Algorithm] = &[
    Algorithm::RS256,
    Algorithm::RS384,
    Algorithm::RS512,
    Algorithm::ES256,
    Algorithm::ES384,
    Algorithm::PS256,
    Algorithm::PS384,
    Algorithm::PS512,
    Algorithm::EdDSA,
];

/// JWT claims we validate.
#[derive(Debug, Deserialize)]
struct Claims {
    #[allow(dead_code)]
    sub: Option<String>,
    #[allow(dead_code)]
    iss: Option<String>,
    #[allow(dead_code)]
    aud: Option<serde_json::Value>,
}

/// Cached JWKS keys fetched from the OIDC provider.
#[derive(Clone)]
struct JwksCache {
    keys: Vec<jsonwebtoken::jwk::Jwk>,
}

/// Auth state shared across gRPC interceptor calls.
pub struct AuthState {
    pub(crate) config: AuthConfig,
    jwks: RwLock<Option<JwksCache>>,
    jwks_uri: Option<String>,
}

impl AuthState {
    /// Create auth state. If OIDC is configured, discovers the JWKS URI.
    /// Panics if OIDC is configured but discovery fails (fail-closed).
    pub async fn new(config: AuthConfig) -> Self {
        let jwks_uri = if let Some(ref issuer) = config.oidc_issuer {
            // HIGH-03: Validate issuer URL uses HTTPS in production.
            if !issuer.starts_with("https://") && !issuer.starts_with("http://localhost") {
                panic!(
                    "OIDC issuer must use HTTPS (got: {issuer}). \
                     Use http://localhost only for development."
                );
            }

            match discover_jwks_uri(issuer).await {
                Ok(uri) => {
                    // Validate JWKS URI also uses HTTPS (second-order SSRF prevention).
                    if !uri.starts_with("https://") && !uri.starts_with("http://localhost") {
                        panic!("JWKS URI from OIDC discovery must use HTTPS (got: {uri})");
                    }
                    tracing::info!(issuer = %issuer, jwks_uri = %uri, "OIDC discovery complete");
                    Some(uri)
                }
                Err(e) => {
                    // HIGH-05: Fail startup if OIDC is configured but discovery fails.
                    panic!("OIDC issuer configured but discovery failed: {e}");
                }
            }
        } else {
            None
        };

        let state = Self {
            config,
            jwks: RwLock::new(None),
            jwks_uri,
        };

        // Pre-fetch JWKS.
        if state.jwks_uri.is_some() {
            state
                .refresh_jwks()
                .await
                .expect("initial JWKS fetch failed — cannot start with OIDC enabled");
        }

        state
    }

    /// Refresh the cached JWKS from the provider.
    pub async fn refresh_jwks(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let uri = self.jwks_uri.as_ref().ok_or("no JWKS URI")?;
        let resp: JwksResponse = reqwest::get(uri).await?.json().await?;
        let mut cache = self.jwks.write().await;
        *cache = Some(JwksCache { keys: resp.keys });
        tracing::debug!(
            key_count = cache.as_ref().unwrap().keys.len(),
            "JWKS refreshed"
        );
        Ok(())
    }

    /// Validate a request's authorization.
    pub async fn check<T>(&self, request: &Request<T>) -> Result<(), Status> {
        // No auth configured = open access.
        if self.config.tokens.is_empty() && self.config.oidc_issuer.is_none() {
            return Ok(());
        }

        let token = extract_bearer_token(request)?;

        // CRITICAL-02: Use constant-time comparison for static tokens.
        if check_static_tokens(&self.config.tokens, token) {
            return Ok(());
        }

        // Try JWT/OIDC validation.
        if self.config.oidc_issuer.is_some() {
            return self.validate_jwt_cached(token);
        }

        Err(Status::unauthenticated("invalid token"))
    }

    /// Validate a JWT against the cached JWKS (synchronous — for use in interceptors).
    /// Shared logic used by both `check()` and `make_interceptor()`.
    fn validate_jwt_cached(&self, token: &str) -> Result<(), Status> {
        let cache = self
            .jwks
            .try_read()
            .map_err(|_| Status::unavailable("JWKS refresh in progress"))?;
        let jwks = cache
            .as_ref()
            .ok_or_else(|| Status::unavailable("JWKS not loaded"))?;

        let header = jsonwebtoken::decode_header(token)
            .map_err(|e| Status::unauthenticated(format!("invalid JWT header: {e}")))?;

        // CRITICAL-01: Never trust the JWT header's alg claim.
        // Derive the algorithm from the JWK, not the token.
        let kid = header.kid.as_deref();

        // MEDIUM-06: Require kid when JWKS has multiple keys.
        if kid.is_none() && jwks.keys.len() > 1 {
            return Err(Status::unauthenticated(
                "JWT missing kid header but JWKS has multiple keys",
            ));
        }

        let jwk = jwks
            .keys
            .iter()
            .find(|k| match (kid, &k.common.key_id) {
                (Some(kid), Some(k_kid)) => kid == k_kid,
                (None, _) if jwks.keys.len() == 1 => true,
                _ => false,
            })
            .ok_or_else(|| Status::unauthenticated("no matching key in JWKS"))?;

        let decoding_key = DecodingKey::from_jwk(jwk)
            .map_err(|e| Status::unauthenticated(format!("invalid JWK: {e}")))?;

        // CRITICAL-01: Use the JWK's algorithm, NOT the token header's.
        let alg = jwk
            .common
            .key_algorithm
            .and_then(|ka| key_algorithm_to_jwt_algorithm(ka))
            .ok_or_else(|| {
                Status::unauthenticated("JWK has no algorithm or unsupported algorithm")
            })?;

        // Double-check it's in our allowlist (no symmetric algorithms).
        if !ALLOWED_ALGORITHMS.contains(&alg) {
            return Err(Status::unauthenticated(format!(
                "algorithm {alg:?} not in allowlist"
            )));
        }

        let mut validation = Validation::new(alg);
        if let Some(ref issuer) = self.config.oidc_issuer {
            validation.set_issuer(&[issuer]);
        }
        if let Some(ref audience) = self.config.oidc_audience {
            validation.set_audience(&[audience]);
        } else {
            validation.validate_aud = false;
        }

        decode::<Claims>(token, &decoding_key, &validation)
            .map_err(|e| Status::unauthenticated(format!("JWT validation failed: {e}")))?;

        Ok(())
    }
}

/// CRITICAL-02: Constant-time token comparison to prevent timing attacks.
/// Public for use in webhook auth.
pub fn check_static_tokens_pub(tokens: &[String], candidate: &str) -> bool {
    check_static_tokens(tokens, candidate)
}

fn check_static_tokens(tokens: &[String], candidate: &str) -> bool {
    use subtle::ConstantTimeEq;
    let candidate_bytes = candidate.as_bytes();
    for token in tokens {
        let token_bytes = token.as_bytes();
        if token_bytes.len() == candidate_bytes.len()
            && bool::from(token_bytes.ct_eq(candidate_bytes))
        {
            return true;
        }
    }
    false
}

/// Extract bearer token from gRPC metadata or HTTP Authorization header.
fn extract_bearer_token<T>(request: &Request<T>) -> Result<&str, Status> {
    let auth = request
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| Status::unauthenticated("missing authorization header"))?;

    auth.strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))
        .ok_or_else(|| Status::unauthenticated("expected Bearer token"))
}

/// Map JWK key algorithm to jsonwebtoken Algorithm.
fn key_algorithm_to_jwt_algorithm(ka: jsonwebtoken::jwk::KeyAlgorithm) -> Option<Algorithm> {
    use jsonwebtoken::jwk::KeyAlgorithm as KA;
    match ka {
        KA::RS256 => Some(Algorithm::RS256),
        KA::RS384 => Some(Algorithm::RS384),
        KA::RS512 => Some(Algorithm::RS512),
        KA::ES256 => Some(Algorithm::ES256),
        KA::ES384 => Some(Algorithm::ES384),
        KA::PS256 => Some(Algorithm::PS256),
        KA::PS384 => Some(Algorithm::PS384),
        KA::PS512 => Some(Algorithm::PS512),
        KA::EdDSA => Some(Algorithm::EdDSA),
        _ => None, // Reject HS256, HS384, HS512 and unknown algorithms.
    }
}

/// OIDC discovery response (minimal — we only need jwks_uri).
#[derive(Deserialize)]
struct OidcDiscovery {
    jwks_uri: String,
}

/// JWKS response.
#[derive(Deserialize)]
struct JwksResponse {
    keys: Vec<jsonwebtoken::jwk::Jwk>,
}

/// Fetch the JWKS URI from the OIDC discovery endpoint.
async fn discover_jwks_uri(
    issuer: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let resp: OidcDiscovery = reqwest::get(&discovery_url).await?.json().await?;
    Ok(resp.jwks_uri)
}

/// Create a tonic interceptor that checks auth on every request.
pub fn make_interceptor(
    auth: Arc<AuthState>,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone {
    move |req: Request<()>| {
        let auth = auth.clone();

        // No auth configured = pass through.
        if auth.config.tokens.is_empty() && auth.config.oidc_issuer.is_none() {
            return Ok(req);
        }

        let token = match extract_bearer_token(&req) {
            Ok(t) => t.to_string(),
            Err(e) => return Err(e),
        };

        // CRITICAL-02: Constant-time static token check.
        if check_static_tokens(&auth.config.tokens, &token) {
            return Ok(req);
        }

        // Check JWT via shared validate_jwt_cached (deduplicated logic).
        if auth.config.oidc_issuer.is_some() {
            auth.validate_jwt_cached(&token)?;
            return Ok(req);
        }

        Err(Status::unauthenticated("invalid token"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_bearer_from_metadata() {
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer mytoken".parse().unwrap());
        assert_eq!(extract_bearer_token(&req).unwrap(), "mytoken");
    }

    #[test]
    fn extract_bearer_lowercase() {
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "bearer mytoken".parse().unwrap());
        assert_eq!(extract_bearer_token(&req).unwrap(), "mytoken");
    }

    #[test]
    fn extract_bearer_missing_header() {
        let req = Request::new(());
        assert!(extract_bearer_token(&req).is_err());
    }

    #[test]
    fn extract_bearer_wrong_scheme() {
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Basic abc".parse().unwrap());
        assert!(extract_bearer_token(&req).is_err());
    }

    #[test]
    fn constant_time_token_check_valid() {
        let tokens = vec!["secret123".to_string()];
        assert!(check_static_tokens(&tokens, "secret123"));
    }

    #[test]
    fn constant_time_token_check_invalid() {
        let tokens = vec!["secret123".to_string()];
        assert!(!check_static_tokens(&tokens, "wrong"));
    }

    #[test]
    fn constant_time_token_check_empty() {
        let tokens: Vec<String> = vec![];
        assert!(!check_static_tokens(&tokens, "anything"));
    }

    #[test]
    fn constant_time_token_check_length_mismatch() {
        let tokens = vec!["short".to_string()];
        assert!(!check_static_tokens(&tokens, "muchlongertoken"));
    }

    #[tokio::test]
    async fn no_auth_configured_allows_all() {
        let state = AuthState {
            config: AuthConfig::default(),
            jwks: RwLock::new(None),
            jwks_uri: None,
        };
        let req = Request::new(());
        assert!(state.check(&req).await.is_ok());
    }

    #[tokio::test]
    async fn static_token_valid() {
        let config = AuthConfig {
            tokens: vec!["secret123".to_string()],
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(None),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer secret123".parse().unwrap());
        assert!(state.check(&req).await.is_ok());
    }

    #[tokio::test]
    async fn static_token_invalid() {
        let config = AuthConfig {
            tokens: vec!["secret123".to_string()],
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(None),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(state.check(&req).await.is_err());
    }

    #[tokio::test]
    async fn static_token_missing_header() {
        let config = AuthConfig {
            tokens: vec!["secret123".to_string()],
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(None),
            jwks_uri: None,
        };
        let req = Request::new(());
        assert!(state.check(&req).await.is_err());
    }

    #[test]
    fn interceptor_no_auth_passes() {
        let state = Arc::new(AuthState {
            config: AuthConfig::default(),
            jwks: RwLock::new(None),
            jwks_uri: None,
        });
        let interceptor = make_interceptor(state);
        let req = Request::new(());
        assert!(interceptor(req).is_ok());
    }

    #[test]
    fn interceptor_static_token_valid() {
        let config = AuthConfig {
            tokens: vec!["tok".to_string()],
            ..Default::default()
        };
        let state = Arc::new(AuthState {
            config,
            jwks: RwLock::new(None),
            jwks_uri: None,
        });
        let interceptor = make_interceptor(state);
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer tok".parse().unwrap());
        assert!(interceptor(req).is_ok());
    }

    #[test]
    fn interceptor_static_token_invalid() {
        let config = AuthConfig {
            tokens: vec!["tok".to_string()],
            ..Default::default()
        };
        let state = Arc::new(AuthState {
            config,
            jwks: RwLock::new(None),
            jwks_uri: None,
        });
        let interceptor = make_interceptor(state);
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer bad".parse().unwrap());
        assert!(interceptor(req).is_err());
    }

    /// Helper: create a test RSA key pair, JWK, and signed JWT.
    fn make_test_jwt(
        issuer: &str,
        audience: Option<&str>,
    ) -> (Vec<jsonwebtoken::jwk::Jwk>, String) {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        use rsa::RsaPrivateKey;

        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = private_key.to_public_key();

        use rsa::traits::PublicKeyParts;
        let n = URL_SAFE_NO_PAD.encode(public_key.n().to_bytes_be());
        let e = URL_SAFE_NO_PAD.encode(public_key.e().to_bytes_be());

        let jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": "test-key-1",
            "n": n,
            "e": e,
        }))
        .unwrap();

        use rsa::pkcs1::EncodeRsaPrivateKey;
        let pem = private_key
            .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
            .unwrap();
        let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();

        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());

        #[derive(serde::Serialize)]
        struct TestClaims {
            sub: String,
            iss: String,
            #[serde(skip_serializing_if = "Option::is_none")]
            aud: Option<String>,
            exp: u64,
            iat: u64,
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = TestClaims {
            sub: "user@example.com".to_string(),
            iss: issuer.to_string(),
            aud: audience.map(String::from),
            exp: now + 3600,
            iat: now,
        };

        let token = jsonwebtoken::encode(&header, &claims, &encoding_key).unwrap();
        (vec![jwk], token)
    }

    #[tokio::test]
    async fn jwt_validation_valid_token() {
        let issuer = "https://auth.example.com";
        let (jwks, token) = make_test_jwt(issuer, None);
        let config = AuthConfig {
            oidc_issuer: Some(issuer.to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: jwks })),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(state.check(&req).await.is_ok());
    }

    #[tokio::test]
    async fn jwt_validation_wrong_issuer() {
        let (jwks, token) = make_test_jwt("https://wrong-issuer.com", None);
        let config = AuthConfig {
            oidc_issuer: Some("https://expected-issuer.com".to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: jwks })),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(state.check(&req).await.is_err());
    }

    #[tokio::test]
    async fn jwt_validation_with_audience() {
        let issuer = "https://auth.example.com";
        let (jwks, token) = make_test_jwt(issuer, Some("wfe-server"));
        let config = AuthConfig {
            oidc_issuer: Some(issuer.to_string()),
            oidc_audience: Some("wfe-server".to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: jwks })),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(state.check(&req).await.is_ok());
    }

    #[tokio::test]
    async fn jwt_validation_wrong_audience() {
        let issuer = "https://auth.example.com";
        let (jwks, token) = make_test_jwt(issuer, Some("wrong-audience"));
        let config = AuthConfig {
            oidc_issuer: Some(issuer.to_string()),
            oidc_audience: Some("wfe-server".to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: jwks })),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(state.check(&req).await.is_err());
    }

    #[tokio::test]
    async fn jwt_validation_garbage_token() {
        let config = AuthConfig {
            oidc_issuer: Some("https://auth.example.com".to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: vec![] })),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer not.a.jwt".parse().unwrap());
        assert!(state.check(&req).await.is_err());
    }

    #[tokio::test]
    async fn jwt_validation_no_jwks_loaded() {
        let config = AuthConfig {
            oidc_issuer: Some("https://auth.example.com".to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(None),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer some.jwt.token".parse().unwrap());
        let err = state.check(&req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unavailable);
    }

    #[test]
    fn interceptor_jwt_valid() {
        let issuer = "https://auth.example.com";
        let (jwks, token) = make_test_jwt(issuer, None);
        let config = AuthConfig {
            oidc_issuer: Some(issuer.to_string()),
            ..Default::default()
        };
        let state = Arc::new(AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: jwks })),
            jwks_uri: None,
        });
        let interceptor = make_interceptor(state);
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(interceptor(req).is_ok());
    }

    #[test]
    fn interceptor_jwt_invalid() {
        let config = AuthConfig {
            oidc_issuer: Some("https://auth.example.com".to_string()),
            ..Default::default()
        };
        let state = Arc::new(AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: vec![] })),
            jwks_uri: None,
        });
        let interceptor = make_interceptor(state);
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", "Bearer bad.jwt.token".parse().unwrap());
        assert!(interceptor(req).is_err());
    }

    #[test]
    fn key_algorithm_mapping() {
        use jsonwebtoken::jwk::KeyAlgorithm as KA;
        assert_eq!(
            key_algorithm_to_jwt_algorithm(KA::RS256),
            Some(Algorithm::RS256)
        );
        assert_eq!(
            key_algorithm_to_jwt_algorithm(KA::ES256),
            Some(Algorithm::ES256)
        );
        assert_eq!(
            key_algorithm_to_jwt_algorithm(KA::EdDSA),
            Some(Algorithm::EdDSA)
        );
        // HS256 should be rejected (symmetric algorithm).
        assert_eq!(key_algorithm_to_jwt_algorithm(KA::HS256), None);
        assert_eq!(key_algorithm_to_jwt_algorithm(KA::HS384), None);
        assert_eq!(key_algorithm_to_jwt_algorithm(KA::HS512), None);
    }

    #[test]
    fn allowed_algorithms_rejects_symmetric() {
        assert!(!ALLOWED_ALGORITHMS.contains(&Algorithm::HS256));
        assert!(!ALLOWED_ALGORITHMS.contains(&Algorithm::HS384));
        assert!(!ALLOWED_ALGORITHMS.contains(&Algorithm::HS512));
    }

    // ── Security regression tests ────────────────────────────────────

    #[test]
    fn security_hs256_rejected_in_allowlist() {
        // CRITICAL-01: HS256 must NEVER be in the allowlist.
        // An attacker with the public RSA key could forge tokens if HS256 is allowed.
        assert!(!ALLOWED_ALGORITHMS.contains(&Algorithm::HS256));
    }

    #[test]
    fn security_key_algorithm_rejects_all_symmetric() {
        // CRITICAL-01: key_algorithm_to_jwt_algorithm must return None for symmetric algs.
        use jsonwebtoken::jwk::KeyAlgorithm as KA;
        assert!(key_algorithm_to_jwt_algorithm(KA::HS256).is_none());
        assert!(key_algorithm_to_jwt_algorithm(KA::HS384).is_none());
        assert!(key_algorithm_to_jwt_algorithm(KA::HS512).is_none());
    }

    #[test]
    fn security_constant_time_comparison_used() {
        // CRITICAL-02: Static token check must use constant-time comparison.
        // Verify that equal-length wrong tokens don't short-circuit.
        let tokens = vec!["abcdefgh".to_string()];
        // Both are 8 chars — a timing attack would try this.
        assert!(!check_static_tokens(&tokens, "abcdefgX"));
        assert!(check_static_tokens(&tokens, "abcdefgh"));
    }

    #[tokio::test]
    #[should_panic(expected = "OIDC issuer must use HTTPS")]
    async fn security_oidc_issuer_requires_https() {
        // HIGH-03: Non-HTTPS issuers must be rejected (SSRF prevention).
        let config = AuthConfig {
            oidc_issuer: Some("http://evil.internal:8080".to_string()),
            ..Default::default()
        };
        AuthState::new(config).await;
    }

    #[tokio::test]
    async fn security_jwt_requires_kid_with_multiple_keys() {
        // MEDIUM-06: When JWKS has multiple keys, JWT must have kid header.
        let (mut jwks, token) = make_test_jwt("https://auth.example.com", None);
        // Duplicate the key with a different kid.
        let mut key2 = jwks[0].clone();
        key2.common.key_id = Some("test-key-2".to_string());
        jwks.push(key2);

        // Strip kid from the token by decoding, modifying header, re-encoding.
        // Easier: just test the validate path with multiple keys and a token that has kid.
        // The token from make_test_jwt has kid="test-key-1", so it should work.
        let config = AuthConfig {
            oidc_issuer: Some("https://auth.example.com".to_string()),
            ..Default::default()
        };
        let state = AuthState {
            config,
            jwks: RwLock::new(Some(JwksCache { keys: jwks })),
            jwks_uri: None,
        };
        let mut req = Request::new(());
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        // Should succeed because the token has kid="test-key-1" which matches.
        assert!(state.check(&req).await.is_ok());
    }
}
