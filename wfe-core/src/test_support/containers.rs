//! Testcontainers-backed service helpers for integration suites.
//!
//! Containers start on whatever Docker endpoint the environment resolves —
//! including the workspace's remote TLS daemon — and published ports dial the
//! daemon host, so suites stay hermetic wherever the daemon lives. When
//! `DOCKER_HOST` is not already exported (direnv usually does it), the active
//! `docker` context is mirrored into the env vars testcontainers reads.
//!
//! Mirrors the sdk testing module; see also `init_docker_host`.

use std::sync::Once;
use std::time::Duration;

use testcontainers::core::{ContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// Pin the process-wide rustls crypto provider to aws-lc-rs. The workspace
/// dependency graph compiles both backends (kube pulls ring, sqlx pulls
/// aws-lc-rs), so rustls cannot auto-select — any TLS handshake, including
/// the one to a remote Docker daemon, would panic without this.
pub fn install_default_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .is_err()
        {
            eprintln!("rustls crypto provider already installed; keeping existing default");
        }
    });
}

/// Prepare the container-test environment before any container starts.
///
/// Ensures `DOCKER_HOST` points at the active Docker context when it is not
/// already set (testcontainers reads `DOCKER_HOST` directly; the Docker CLI's
/// context-aware endpoint is not picked up on its own). For TLS-secured
/// remote daemons the context's CA/cert/key material is mirrored into
/// `DOCKER_CERT_PATH` / `DOCKER_TLS_VERIFY` and the endpoint rewritten to
/// `https://`. Also pins the rustls crypto provider (see
/// [`install_default_crypto_provider`]).
pub fn init_docker_host() {
    install_default_crypto_provider();

    if std::env::var("DOCKER_HOST").is_ok() {
        return;
    }

    let output = std::process::Command::new("docker")
        .args([
            "context",
            "inspect",
            "-f",
            "{{.Endpoints.docker.Host}}\n{{.Storage.TLSPath}}",
        ])
        .output();

    let (mut host, tls_dir) = match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            // Missing lines mean an unusual context shape; the emptiness
            // handling below falls back to the default socket.
            let host = lines.next().unwrap_or("").to_string();
            let tls = lines.next().unwrap_or("").to_string();
            (host, tls)
        }
        _ => (String::new(), String::new()),
    };

    if host.is_empty() {
        // Fall back to the default unix socket. If Docker isn't there,
        // testcontainers fails with a clear connection error.
        // SAFETY: single-threaded test-process init; env vars are read later
        // by testcontainers when the first container starts.
        unsafe { std::env::set_var("DOCKER_HOST", "unix:///var/run/docker.sock") };
        return;
    }

    let ca = std::path::Path::new(&tls_dir).join("docker").join("ca.pem");
    if host.starts_with("tcp://") && ca.exists() && std::env::var_os("DOCKER_CERT_PATH").is_none() {
        // SAFETY: same single-threaded test-init window as above.
        unsafe {
            std::env::set_var("DOCKER_CERT_PATH", ca.parent().unwrap());
            std::env::set_var("DOCKER_TLS_VERIFY", "1");
        }
        host = host.replacen("tcp://", "https://", 1);
    }

    // SAFETY: same single-threaded test-init window as above.
    unsafe { std::env::set_var("DOCKER_HOST", host) };
}

/// Testcontainers builder for PostgreSQL with WFE-friendly defaults.
#[derive(Debug, Clone)]
pub struct Postgres {
    tag: String,
    user: String,
    password: String,
    db: String,
}

impl Default for Postgres {
    fn default() -> Self {
        Self {
            tag: "16".to_owned(),
            user: "wfe".to_owned(),
            password: "wfe".to_owned(),
            db: "wfe_test".to_owned(),
        }
    }
}

impl Postgres {
    /// PostgreSQL server port.
    pub const PORT: u16 = 5432;

    /// Create a builder with the default configuration (postgres:16,
    /// `wfe`/`wfe`/`wfe_test`).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the image tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }

    /// Override the default credentials and database name.
    pub fn with_credentials(
        mut self,
        user: impl Into<String>,
        password: impl Into<String>,
        db: impl Into<String>,
    ) -> Self {
        self.user = user.into();
        self.password = password.into();
        self.db = db.into();
        self
    }

    /// Start a container with its port published on a random host port.
    pub async fn start(
        self,
    ) -> Result<ContainerAsync<GenericImage>, testcontainers::TestcontainersError> {
        init_docker_host();
        GenericImage::new("postgres", &self.tag)
            .with_exposed_port(ContainerPort::Tcp(Self::PORT))
            .with_wait_for(WaitFor::message_on_stdout(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_USER", self.user)
            .with_env_var("POSTGRES_PASSWORD", self.password)
            .with_env_var("POSTGRES_DB", self.db)
            .with_startup_timeout(Duration::from_secs(120))
            .with_mapped_port(0, ContainerPort::Tcp(Self::PORT))
            .start()
            .await
    }

    /// Connection URL for a container started via [`Postgres::start`].
    pub async fn url(
        container: &ContainerAsync<GenericImage>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let host = container.get_host().await?.to_string();
        let port = container
            .get_host_port_ipv4(ContainerPort::Tcp(Self::PORT))
            .await?;
        Ok(format!("postgres://wfe:wfe@{host}:{port}/wfe_test"))
    }
}

/// Testcontainers builder for Valkey (Redis-compatible).
#[derive(Debug, Clone)]
pub struct Valkey {
    tag: String,
}

impl Default for Valkey {
    fn default() -> Self {
        Self {
            tag: "8-alpine".to_owned(),
        }
    }
}

impl Valkey {
    /// Valkey server port.
    pub const PORT: u16 = 6379;

    /// Create a builder with the default configuration (valkey/valkey:8-alpine).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the image tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }

    /// Start a container with its port published on a random host port.
    pub async fn start(
        self,
    ) -> Result<ContainerAsync<GenericImage>, testcontainers::TestcontainersError> {
        init_docker_host();
        GenericImage::new("valkey/valkey", &self.tag)
            .with_exposed_port(ContainerPort::Tcp(Self::PORT))
            .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
            .with_startup_timeout(Duration::from_secs(120))
            .with_mapped_port(0, ContainerPort::Tcp(Self::PORT))
            .start()
            .await
    }

    /// Connection URL for a container started via [`Valkey::start`].
    pub async fn url(
        container: &ContainerAsync<GenericImage>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let host = container.get_host().await?.to_string();
        let port = container
            .get_host_port_ipv4(ContainerPort::Tcp(Self::PORT))
            .await?;
        Ok(format!("redis://{host}:{port}"))
    }
}
