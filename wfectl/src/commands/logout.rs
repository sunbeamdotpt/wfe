//! `wfectl logout` -- delete cached OIDC token.

use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config;

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// OIDC issuer to log out from (defaults to configured issuer).
    #[arg(long)]
    pub issuer: Option<String>,
}

pub async fn run(args: LogoutArgs, server_cfg: &config::Config) -> Result<()> {
    let issuer = args.issuer.unwrap_or_else(|| server_cfg.issuer.clone());
    let domain = auth::domain_from_issuer(&issuer)?;

    let deleted = auth::delete_token(&domain)?;
    if deleted {
        println!("✓ Logged out of {domain}");
    } else {
        println!("Not logged in to {domain} (no token cache found)");
    }
    Ok(())
}
