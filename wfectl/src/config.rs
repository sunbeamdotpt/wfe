//! Persistent configuration loaded from `~/.config/wfectl/config.toml`.
//!
//! Resolution precedence: CLI flag > env var > config file > built-in default.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default wfe-server endpoint (Pingora terminates TLS, h2c upstream).
pub const DEFAULT_SERVER: &str = "https://builds.sunbeam.pt:443";
/// Default OIDC issuer.
pub const DEFAULT_ISSUER: &str = "https://auth.sunbeam.pt/";

/// Persisted user configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_server")]
    /// Server.
    pub server: String,
    #[serde(default = "default_issuer")]
    /// Issuer.
    pub issuer: String,
    #[serde(default)]
    /// Default format.
    pub default_format: OutputFormatPref,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: default_server(),
            issuer: default_issuer(),
            default_format: OutputFormatPref::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// Outputformatpref.
pub enum OutputFormatPref {
    #[default]
    /// Table.
    Table,
    /// Json.
    Json,
}

fn default_server() -> String {
    DEFAULT_SERVER.into()
}
fn default_issuer() -> String {
    DEFAULT_ISSUER.into()
}

/// Path to the config file.
pub fn config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    base.join("wfectl/config.toml")
}

/// Load the config file. Returns the default config if the file doesn't exist.
pub fn load() -> Result<Config> {
    let path = config_path();
    if !path.exists() {
        return Ok(Config::default());
    }
    let bytes = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config at {}", path.display()))?;
    let config: Config = toml::from_str(&bytes)
        .with_context(|| format!("failed to parse config at {}", path.display()))?;
    Ok(config)
}

/// Save the config to disk, creating parent directories as needed.
pub fn save(config: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config dir {}", parent.display()))?;
    }
    let toml = toml::to_string_pretty(config).context("failed to serialize config")?;
    std::fs::write(&path, toml)
        .with_context(|| format!("failed to write config to {}", path.display()))?;
    Ok(())
}

/// Resolve a setting with CLI > env > file > default precedence.
pub fn resolve<T: AsRef<str>>(cli: Option<T>, env_key: &str, file_value: &str) -> String {
    if let Some(v) = cli {
        return v.as_ref().to_string();
    }
    if let Ok(v) = std::env::var(env_key) {
        if !v.is_empty() {
            return v;
        }
    }
    file_value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn config_default_values() {
        let config = Config::default();
        assert_eq!(config.server, DEFAULT_SERVER);
        assert_eq!(config.issuer, DEFAULT_ISSUER);
        assert_eq!(config.default_format, OutputFormatPref::Table);
    }

    #[test]
    fn config_serde_round_trip() {
        let original = Config {
            server: "http://localhost:50051".into(),
            issuer: "https://auth.dev.com/".into(),
            default_format: OutputFormatPref::Json,
        };
        let toml_str = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.server, original.server);
        assert_eq!(parsed.issuer, original.issuer);
        assert_eq!(parsed.default_format, original.default_format);
    }

    #[test]
    fn config_partial_uses_defaults() {
        let toml_str = r#"server = "http://other:50051""#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server, "http://other:50051");
        assert_eq!(config.issuer, DEFAULT_ISSUER);
    }

    #[test]
    fn resolve_prefers_cli() {
        unsafe { std::env::set_var("WFECTL_TEST_X", "from_env") };
        let v = resolve(Some("from_cli"), "WFECTL_TEST_X", "from_file");
        assert_eq!(v, "from_cli");
        unsafe { std::env::remove_var("WFECTL_TEST_X") };
    }

    #[test]
    fn resolve_prefers_env_when_no_cli() {
        unsafe { std::env::set_var("WFECTL_TEST_Y", "from_env") };
        let v = resolve(None::<&str>, "WFECTL_TEST_Y", "from_file");
        assert_eq!(v, "from_env");
        unsafe { std::env::remove_var("WFECTL_TEST_Y") };
    }

    #[test]
    fn resolve_falls_back_to_file() {
        unsafe { std::env::remove_var("WFECTL_TEST_Z") };
        let v = resolve(None::<&str>, "WFECTL_TEST_Z", "from_file");
        assert_eq!(v, "from_file");
    }

    #[test]
    fn resolve_treats_empty_env_as_unset() {
        unsafe { std::env::set_var("WFECTL_TEST_EMPTY", "") };
        let v = resolve(None::<&str>, "WFECTL_TEST_EMPTY", "from_file");
        assert_eq!(v, "from_file");
        unsafe { std::env::remove_var("WFECTL_TEST_EMPTY") };
    }

    #[test]
    fn save_load_round_trip_in_temp_home() {
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };

        let config = Config {
            server: "http://localhost:50051".into(),
            issuer: "https://auth.local/".into(),
            default_format: OutputFormatPref::Json,
        };

        save(&config).unwrap();
        let loaded = load().unwrap();
        assert_eq!(loaded.server, "http://localhost:50051");
        assert_eq!(loaded.default_format, OutputFormatPref::Json);
    }

    #[test]
    fn load_returns_default_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("XDG_CONFIG_HOME", tmp.path()) };
        let config = load().unwrap();
        // Should match defaults.
        assert_eq!(config.server, DEFAULT_SERVER);
    }
}
