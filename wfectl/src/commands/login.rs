//! `wfectl login` -- run OAuth2 PKCE flow against the configured OIDC issuer.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use clap::Args;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use indicatif::{ProgressBar, ProgressStyle};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::auth::{
    self, CALLBACK_PORTS, Pkce, build_auth_url, discover, exchange_code, open_browser,
    random_state, save_token,
};
use crate::config;

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// OIDC issuer URL (e.g., https://auth.sunbeam.pt/).
    #[arg(long)]
    pub issuer: Option<String>,
}

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

const SUCCESS_HTML: &str = r#"<!doctype html>
<html><head><title>wfectl login</title></head>
<body style="font-family:system-ui;text-align:center;padding:4rem">
<h1>You're logged in.</h1>
<p>You can close this window and return to the terminal.</p>
</body></html>"#;

const ERROR_HTML: &str = r#"<!doctype html>
<html><head><title>wfectl login error</title></head>
<body style="font-family:system-ui;text-align:center;padding:4rem">
<h1>Login failed</h1>
<p>See the terminal for details.</p>
</body></html>"#;

pub async fn run(args: LoginArgs, server_cfg: &config::Config) -> Result<()> {
    let issuer = args.issuer.unwrap_or_else(|| server_cfg.issuer.clone());
    let domain = auth::domain_from_issuer(&issuer)?;

    println!("Discovering OIDC endpoints at {issuer}...");
    let discovery = discover(&issuer).await?;

    let pkce = Pkce::generate();
    let state = random_state();

    // Bind a callback listener on the first available port.
    let (listener, port) = bind_callback_listener().await?;
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let auth_url = build_auth_url(&discovery, &redirect_uri, &state, &pkce.challenge);

    println!();
    println!("Opening browser for authentication...");
    println!("If your browser doesn't open, visit:");
    println!("  {auth_url}");
    println!();

    open_browser(&auth_url);

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap(),
    );
    progress.set_message("Waiting for authorization callback...");
    progress.enable_steady_tick(Duration::from_millis(100));

    let result =
        tokio::time::timeout(CALLBACK_TIMEOUT, await_callback(listener, state.clone())).await;
    progress.finish_and_clear();

    let callback = match result {
        Ok(Ok(cb)) => cb,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(anyhow!(
                "login timed out after {} seconds",
                CALLBACK_TIMEOUT.as_secs()
            ));
        }
    };

    println!("Got authorization code, exchanging for tokens...");
    let token = exchange_code(
        &discovery,
        &callback.code,
        &pkce.verifier,
        &redirect_uri,
        &issuer,
        &domain,
    )
    .await?;

    save_token(&token)?;
    println!();
    println!("✓ Logged in to {domain} (token cached at ~/.sunbeam/auth/{domain}.json)");
    if let Some(claims) = token.id_claims() {
        if let Some(email) = claims.get("email").and_then(|v| v.as_str()) {
            println!("  Identity: {email}");
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Callback {
    code: String,
}

async fn bind_callback_listener() -> Result<(TcpListener, u16)> {
    for port in CALLBACK_PORTS {
        let addr = format!("127.0.0.1:{port}");
        if let Ok(listener) = TcpListener::bind(&addr).await {
            return Ok((listener, port));
        }
    }
    // Fall back to ephemeral.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("failed to bind callback listener on any port")?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

async fn await_callback(listener: TcpListener, expected_state: String) -> Result<Callback> {
    let (tx, rx) = oneshot::channel::<Result<Callback>>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };

            let tx = tx.clone();
            let expected = expected_state.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                    let tx = tx.clone();
                    let expected = expected.clone();
                    async move { handle_callback_request(req, tx, expected).await }
                });
                let _ = http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    });

    rx.await.context("callback channel closed")?
}

async fn handle_callback_request(
    req: Request<hyper::body::Incoming>,
    tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<Result<Callback>>>>>,
    expected_state: String,
) -> Result<Response<Full<Bytes>>, Infallible> {
    if !req.uri().path().starts_with("/callback") {
        return Ok(Response::builder()
            .status(404)
            .body(Full::new(Bytes::new()))
            .unwrap());
    }

    let query = req.uri().query().unwrap_or("");
    let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();

    let result = match (params.get("code"), params.get("state"), params.get("error")) {
        (_, _, Some(err)) => Err(anyhow!("OAuth error: {err}")),
        (Some(code), Some(state), None) if state == &expected_state => {
            Ok(Callback { code: code.clone() })
        }
        (_, Some(_), None) => Err(anyhow!("OAuth state mismatch (possible CSRF)")),
        _ => Err(anyhow!("missing code or state in callback")),
    };

    let (status, body) = if result.is_ok() {
        (200, SUCCESS_HTML)
    } else {
        (400, ERROR_HTML)
    };

    // Send the result through the oneshot, taking it out of the Mutex.
    {
        let mut guard = tx.lock().await;
        if let Some(sender) = guard.take() {
            let _ = sender.send(result);
        }
    }

    Ok(Response::builder()
        .status(status)
        .header("content-type", "text/html; charset=utf-8")
        .body(Full::new(Bytes::from(body)))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_callback_listener_succeeds_on_some_port() {
        let (listener, port) = bind_callback_listener().await.unwrap();
        assert!(port > 0);
        drop(listener);
    }

    /// Drive the full callback path: bind listener, send a request, observe result.
    async fn drive_callback(query: &str) -> Result<Callback> {
        let (listener, port) = bind_callback_listener().await.unwrap();
        let state = "test-state";

        // Spawn the await_callback future.
        let handle =
            tokio::spawn(async move { await_callback(listener, "test-state".to_string()).await });

        // Give it a moment to start the accept loop.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Send a request.
        let url = format!("http://127.0.0.1:{port}/callback?{query}");
        let _ = reqwest::get(&url).await;

        // Drop unused state to avoid warnings.
        let _ = state;

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn callback_with_valid_code_and_state_succeeds() {
        let result = drive_callback("code=abc123&state=test-state").await;
        let cb = result.unwrap();
        assert_eq!(cb.code, "abc123");
    }

    #[tokio::test]
    async fn callback_with_state_mismatch_fails() {
        let result = drive_callback("code=abc&state=wrong-state").await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("state mismatch"));
    }

    #[tokio::test]
    async fn callback_with_oauth_error_fails() {
        let result = drive_callback("error=access_denied&state=test-state").await;
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("access_denied"));
    }

    #[tokio::test]
    async fn callback_with_missing_params_fails() {
        let result = drive_callback("nothing=here").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn callback_404_for_non_callback_path() {
        // This tests the 404 branch in handle_callback_request.
        let (listener, port) = bind_callback_listener().await.unwrap();
        let _handle = tokio::spawn(async move {
            let _ = await_callback(listener, "s".to_string()).await;
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let resp = reqwest::get(format!("http://127.0.0.1:{port}/not-callback"))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }
}
