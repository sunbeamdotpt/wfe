//! Tonic gRPC client wrapper with bearer-token authentication.

use anyhow::{Context, Result, anyhow};
use tonic::metadata::MetadataValue;
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use wfe_server_protos::wfe::v1::wfe_client::WfeClient as GeneratedWfeClient;

/// Type alias for the fully-instantiated wfe client with auth interceptor.
pub type AuthClient = GeneratedWfeClient<InterceptedService<Channel, BearerAuth>>;

/// Tonic interceptor that injects an `Authorization: Bearer <token>` header
/// on every gRPC request.
#[derive(Clone)]
pub struct BearerAuth {
    header: Option<MetadataValue<tonic::metadata::Ascii>>,
}

impl BearerAuth {
    /// Construct a new bearer-auth interceptor. An empty token results in
    /// no header being injected (useful for unauthenticated calls).
    pub fn new(token: &str) -> Result<Self> {
        if token.is_empty() {
            return Ok(Self { header: None });
        }
        let value = format!("Bearer {token}");
        let header = MetadataValue::try_from(value)
            .map_err(|e| anyhow!("invalid auth token (cannot encode as header): {e}"))?;
        Ok(Self {
            header: Some(header),
        })
    }
}

impl Interceptor for BearerAuth {
    fn call(&mut self, mut req: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(header) = &self.header {
            req.metadata_mut().insert("authorization", header.clone());
        }
        Ok(req)
    }
}

/// Build a tonic channel for the given server URL, configuring TLS automatically
/// when the URL scheme is `https`.
pub async fn connect(server: &str) -> Result<Channel> {
    let mut endpoint = Endpoint::from_shared(server.to_string())
        .with_context(|| format!("invalid server URL: {server}"))?;

    if server.starts_with("https://") {
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().with_native_roots())
            .context("failed to configure TLS")?;
    }

    endpoint
        .connect()
        .await
        .with_context(|| format!("failed to connect to {server}"))
}

/// Build an authenticated wfe client.
pub async fn build(server: &str, token: &str) -> Result<AuthClient> {
    let channel = connect(server).await?;
    let auth = BearerAuth::new(token)?;
    Ok(GeneratedWfeClient::with_interceptor(channel, auth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_auth_with_empty_token_injects_nothing() {
        let mut auth = BearerAuth::new("").unwrap();
        let req = tonic::Request::new(());
        let out = auth.call(req).unwrap();
        assert!(out.metadata().get("authorization").is_none());
    }

    #[test]
    fn bearer_auth_injects_header() {
        let mut auth = BearerAuth::new("ory_at_xyz").unwrap();
        let req = tonic::Request::new(());
        let out = auth.call(req).unwrap();
        let header = out.metadata().get("authorization").unwrap();
        assert_eq!(header.to_str().unwrap(), "Bearer ory_at_xyz");
    }

    #[test]
    fn bearer_auth_rejects_invalid_chars() {
        // Tokens containing newlines can't be encoded as HTTP headers.
        let result = BearerAuth::new("bad\ntoken");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_invalid_url_returns_error() {
        let result = connect("not a valid url").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn connect_to_unreachable_address_fails() {
        let result = connect("http://127.0.0.1:1").await;
        assert!(result.is_err());
    }
}
