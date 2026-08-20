use async_nats::ConnectOptions;

use crate::config::{NatsAuthConfig, NatsConfig};

/// Build `ConnectOptions` from the provided auth configuration.
pub async fn connect_options(config: &NatsConfig) -> wfe_core::Result<ConnectOptions> {
    let opts = match &config.auth {
        NatsAuthConfig::None => ConnectOptions::new(),
        NatsAuthConfig::Credentials { path } => ConnectOptions::with_credentials_file(path)
            .await
            .map_err(|e| wfe_core::WfeError::Other(format!("invalid credentials file: {e}").into()))?,
        NatsAuthConfig::Jwt { jwt, nkey } => {
            let creds = format_credentials(jwt, nkey);
            ConnectOptions::with_credentials(&creds)
                .map_err(|e| wfe_core::WfeError::Other(format!("invalid jwt/nkey: {e}").into()))?
        }
        NatsAuthConfig::UserPassword { user, password } => {
            ConnectOptions::with_user_and_password(user.clone(), password.clone())
        }
        NatsAuthConfig::Token { token } => ConnectOptions::with_token(token.clone()),
    };

    Ok(opts)
}

/// Resolve a server URL string into a `ConnectOptions::connect` invocation.
pub async fn connect(config: &NatsConfig) -> wfe_core::Result<async_nats::Client> {
    let opts = connect_options(config).await?;
    opts.connect(&config.url)
        .await
        .map_err(|e| wfe_core::WfeError::Persistence(format!("nats connect failed: {e}")))
}

/// Format a JWT and nkey seed into the standard NATS credentials string.
fn format_credentials(jwt: &str, nkey: &str) -> String {
    format!(
        "-----BEGIN NATS USER JWT-----\n{}\n------END NATS USER JWT------\n\n\
         ************************* IMPORTANT *************************\n\
         NKEY Seed printed below can be used sign and prove identity.\n\
         NKEYs are sensitive and should be treated as secrets.\n\n\
         -----BEGIN USER NKEY SEED-----\n{}\n------END USER NKEY SEED------\n",
        jwt.trim(),
        nkey.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_credentials_includes_jwt_and_seed() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";
        let nkey = "SUAIQDHXV6NBOCYPYJWX7XPHXCEKQ7PXWWLJ2UCLD5M7S7BQZR5NBO4K";
        let creds = format_credentials(jwt, nkey);
        assert!(creds.contains("BEGIN NATS USER JWT"));
        assert!(creds.contains(jwt));
        assert!(creds.contains("BEGIN USER NKEY SEED"));
        assert!(creds.contains(nkey));
    }
}
