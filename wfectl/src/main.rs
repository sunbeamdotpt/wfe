//! wfectl: command-line client for wfe-server.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use wfectl::client::build as build_client;
use wfectl::commands::{
    auth::resolve_token, cancel, definitions, get, list, login, logout, logs, publish, register,
    resume, run, search_logs, suspend, validate, watch, whoami,
};
use wfectl::config;
use wfectl::output::OutputFormat;

#[derive(Debug, Parser)]
#[command(
    name = "wfectl",
    version,
    about = "Command-line client for wfe-server",
    long_about = "Authenticate, register, run, monitor, and manage WFE workflows from the terminal."
)]
struct Cli {
    /// Override the wfe-server URL.
    #[arg(long, env = "WFECTL_SERVER", global = true)]
    server: Option<String>,

    /// Override the OIDC issuer URL (used by login/whoami).
    #[arg(long, env = "WFECTL_ISSUER", global = true)]
    issuer: Option<String>,

    /// Bearer token for direct auth (skips OIDC). Falls back to WFECTL_TOKEN env then cached login.
    #[arg(long, env = "WFECTL_TOKEN", global = true)]
    token: Option<String>,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = OutputFormat::Table, global = true)]
    output: OutputFormat,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the OAuth2 PKCE login flow.
    Login(login::LoginArgs),
    /// Delete cached OIDC token.
    Logout(logout::LogoutArgs),
    /// Show current user identity.
    Whoami(whoami::WhoamiArgs),

    /// Register a workflow definition from a YAML file.
    Register(register::RegisterArgs),
    /// Locally validate a workflow YAML file (no server round-trip).
    Validate(validate::ValidateArgs),
    /// Manage registered workflow definitions.
    Definitions(definitions::DefinitionsArgs),

    /// Start a new workflow instance.
    Run(run::RunArgs),
    /// Get a workflow instance by ID.
    Get(get::GetArgs),
    /// List/search workflow instances.
    List(list::ListArgs),
    /// Cancel a running workflow.
    Cancel(cancel::CancelArgs),
    /// Suspend a running workflow.
    Suspend(suspend::SuspendArgs),
    /// Resume a suspended workflow.
    Resume(resume::ResumeArgs),

    /// Publish an event to waiting workflows.
    Publish(publish::PublishArgs),
    /// Stream lifecycle events.
    Watch(watch::WatchArgs),
    /// Stream step logs.
    Logs(logs::LogsArgs),
    /// Full-text search log lines.
    SearchLogs(search_logs::SearchLogsArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let mut cfg = config::load().unwrap_or_default();
    if let Some(s) = cli.server.clone() {
        cfg.server = s;
    }
    if let Some(i) = cli.issuer.clone() {
        cfg.issuer = i;
    }

    match cli.cmd {
        // --- Commands that don't need a gRPC client ---
        Command::Login(args) => login::run(args, &cfg).await,
        Command::Logout(args) => logout::run(args, &cfg).await,
        Command::Whoami(args) => whoami::run(args, &cfg, cli.output).await,
        Command::Validate(args) => validate::run(args, cli.output).await,

        // --- Commands that need an authenticated gRPC client ---
        cmd => {
            let token = resolve_token(cli.token.as_deref(), &cfg.issuer).await?;
            let client = build_client(&cfg.server, &token).await?;
            dispatch(cmd, client, cli.output).await
        }
    }
}

async fn dispatch(
    cmd: Command,
    client: wfectl::client::AuthClient,
    format: OutputFormat,
) -> Result<()> {
    match cmd {
        Command::Register(args) => register::run(args, client, format).await,
        Command::Definitions(args) => definitions::run(args, client, format).await,
        Command::Run(args) => run::run(args, client, format).await,
        Command::Get(args) => get::run(args, client, format).await,
        Command::List(args) => list::run(args, client, format).await,
        Command::Cancel(args) => cancel::run(args, client).await,
        Command::Suspend(args) => suspend::run(args, client).await,
        Command::Resume(args) => resume::run(args, client).await,
        Command::Publish(args) => publish::run(args, client, format).await,
        Command::Watch(args) => watch::run(args, client).await,
        Command::Logs(args) => logs::run(args, client).await,
        Command::SearchLogs(args) => search_logs::run(args, client, format).await,
        Command::Login(_) | Command::Logout(_) | Command::Whoami(_) | Command::Validate(_) => {
            unreachable!()
        }
    }
}
