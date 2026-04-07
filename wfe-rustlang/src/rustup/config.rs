use serde::{Deserialize, Serialize};

/// Which rustup operation to perform.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum RustupCommand {
    /// Install Rust via rustup-init.
    Install,
    /// Install a toolchain (`rustup toolchain install`).
    ToolchainInstall,
    /// Add a component (`rustup component add`).
    ComponentAdd,
    /// Add a compilation target (`rustup target add`).
    TargetAdd,
}

impl RustupCommand {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::ToolchainInstall => "toolchain-install",
            Self::ComponentAdd => "component-add",
            Self::TargetAdd => "target-add",
        }
    }
}

/// Configuration for rustup step types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustupConfig {
    pub command: RustupCommand,
    /// Toolchain to install or scope components/targets to (e.g. "nightly", "1.78.0").
    #[serde(default)]
    pub toolchain: Option<String>,
    /// Components to add (e.g. ["clippy", "rustfmt", "rust-src"]).
    #[serde(default)]
    pub components: Vec<String>,
    /// Compilation targets to add (e.g. ["wasm32-unknown-unknown"]).
    #[serde(default)]
    pub targets: Vec<String>,
    /// Rustup profile for initial install: "minimal", "default", or "complete".
    #[serde(default)]
    pub profile: Option<String>,
    /// Default toolchain to set during install.
    #[serde(default)]
    pub default_toolchain: Option<String>,
    /// Additional arguments appended to the command.
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Execution timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn command_as_str() {
        assert_eq!(RustupCommand::Install.as_str(), "install");
        assert_eq!(
            RustupCommand::ToolchainInstall.as_str(),
            "toolchain-install"
        );
        assert_eq!(RustupCommand::ComponentAdd.as_str(), "component-add");
        assert_eq!(RustupCommand::TargetAdd.as_str(), "target-add");
    }

    #[test]
    fn command_serde_kebab_case() {
        let json = r#""toolchain-install""#;
        let cmd: RustupCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, RustupCommand::ToolchainInstall);

        let serialized = serde_json::to_string(&RustupCommand::ComponentAdd).unwrap();
        assert_eq!(serialized, r#""component-add""#);
    }

    #[test]
    fn serde_round_trip_install() {
        let config = RustupConfig {
            command: RustupCommand::Install,
            toolchain: None,
            components: vec![],
            targets: vec![],
            profile: Some("minimal".to_string()),
            default_toolchain: Some("stable".to_string()),
            extra_args: vec![],
            timeout_ms: Some(300_000),
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: RustupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.command, RustupCommand::Install);
        assert_eq!(de.profile, Some("minimal".to_string()));
        assert_eq!(de.default_toolchain, Some("stable".to_string()));
        assert_eq!(de.timeout_ms, Some(300_000));
    }

    #[test]
    fn serde_round_trip_toolchain_install() {
        let config = RustupConfig {
            command: RustupCommand::ToolchainInstall,
            toolchain: Some("nightly-2024-06-01".to_string()),
            components: vec![],
            targets: vec![],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: RustupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.command, RustupCommand::ToolchainInstall);
        assert_eq!(de.toolchain, Some("nightly-2024-06-01".to_string()));
    }

    #[test]
    fn serde_round_trip_component_add() {
        let config = RustupConfig {
            command: RustupCommand::ComponentAdd,
            toolchain: Some("nightly".to_string()),
            components: vec![
                "clippy".to_string(),
                "rustfmt".to_string(),
                "rust-src".to_string(),
            ],
            targets: vec![],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: RustupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.command, RustupCommand::ComponentAdd);
        assert_eq!(de.components, vec!["clippy", "rustfmt", "rust-src"]);
        assert_eq!(de.toolchain, Some("nightly".to_string()));
    }

    #[test]
    fn serde_round_trip_target_add() {
        let config = RustupConfig {
            command: RustupCommand::TargetAdd,
            toolchain: Some("stable".to_string()),
            components: vec![],
            targets: vec![
                "wasm32-unknown-unknown".to_string(),
                "aarch64-linux-android".to_string(),
            ],
            profile: None,
            default_toolchain: None,
            extra_args: vec![],
            timeout_ms: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: RustupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.command, RustupCommand::TargetAdd);
        assert_eq!(
            de.targets,
            vec!["wasm32-unknown-unknown", "aarch64-linux-android"]
        );
    }

    #[test]
    fn config_defaults() {
        let json = r#"{"command": "install"}"#;
        let config: RustupConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.command, RustupCommand::Install);
        assert!(config.toolchain.is_none());
        assert!(config.components.is_empty());
        assert!(config.targets.is_empty());
        assert!(config.profile.is_none());
        assert!(config.default_toolchain.is_none());
        assert!(config.extra_args.is_empty());
        assert!(config.timeout_ms.is_none());
    }

    #[test]
    fn serde_with_extra_args() {
        let config = RustupConfig {
            command: RustupCommand::ToolchainInstall,
            toolchain: Some("nightly".to_string()),
            components: vec![],
            targets: vec![],
            profile: Some("minimal".to_string()),
            default_toolchain: None,
            extra_args: vec!["--force".to_string()],
            timeout_ms: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: RustupConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.extra_args, vec!["--force"]);
    }
}
