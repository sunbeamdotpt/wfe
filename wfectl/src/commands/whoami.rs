//! `wfectl whoami` -- print current user identity from cached token.

use anyhow::Result;
use clap::Args;

use crate::auth;
use crate::config;
use crate::output::{OutputFormat, render_kv};

#[derive(Debug, Args)]
pub struct WhoamiArgs {
    /// OIDC issuer to inspect (defaults to configured issuer).
    #[arg(long)]
    pub issuer: Option<String>,
}

pub async fn run(
    args: WhoamiArgs,
    server_cfg: &config::Config,
    format: OutputFormat,
) -> Result<()> {
    let issuer = args.issuer.unwrap_or_else(|| server_cfg.issuer.clone());
    let domain = auth::domain_from_issuer(&issuer)?;

    let token = match auth::load_token(&domain)? {
        Some(t) => t,
        None => {
            println!("Not logged in to {domain}. Run `wfectl login` first.");
            return Ok(());
        }
    };

    let claims = token.id_claims();

    if matches!(format, OutputFormat::Json) {
        let json = serde_json::json!({
            "domain": token.domain,
            "issuer": token.issuer,
            "expires_at": token.expires_at,
            "claims": claims,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    let mut fields = vec![
        ("Domain", token.domain.clone()),
        ("Issuer", token.issuer.clone()),
        ("Expires", crate::output::fmt_time(&token.expires_at)),
    ];
    if let Some(claims) = &claims {
        if let Some(email) = claims.get("email").and_then(|v| v.as_str()) {
            fields.push(("Email", email.to_string()));
        }
        if let Some(name) = claims.get("name").and_then(|v| v.as_str()) {
            fields.push(("Name", name.to_string()));
        }
        if let Some(groups) = claims.get("groups").and_then(|v| v.as_array()) {
            let joined = groups
                .iter()
                .filter_map(|g| g.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            fields.push(("Groups", joined));
        }
    }
    let display: Vec<(&str, String)> = fields.iter().map(|(k, v)| (*k, v.clone())).collect();
    println!("{}", render_kv(&display));
    Ok(())
}
