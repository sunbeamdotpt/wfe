use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Which cargo subcommand to run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CargoCommand {
    Build,
    Test,
    Check,
    Clippy,
    Fmt,
    Doc,
    Publish,
    Audit,
    Deny,
    Nextest,
    LlvmCov,
    DocMdx,
}

impl CargoCommand {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Test => "test",
            Self::Check => "check",
            Self::Clippy => "clippy",
            Self::Fmt => "fmt",
            Self::Doc => "doc",
            Self::Publish => "publish",
            Self::Audit => "audit",
            Self::Deny => "deny",
            Self::Nextest => "nextest",
            Self::LlvmCov => "llvm-cov",
            Self::DocMdx => "doc-mdx",
        }
    }

    /// Returns the subcommand arg(s) to pass to cargo.
    /// Most commands are a single arg, but nextest needs "nextest run".
    /// DocMdx uses `rustdoc` (the actual cargo subcommand).
    pub fn subcommand_args(&self) -> Vec<&'static str> {
        match self {
            Self::Nextest => vec!["nextest", "run"],
            Self::DocMdx => vec!["rustdoc"],
            other => vec![other.as_str()],
        }
    }

    /// Returns the cargo-install package name if this is an external tool.
    /// Returns `None` for built-in cargo subcommands.
    pub fn install_package(&self) -> Option<&'static str> {
        match self {
            Self::Audit => Some("cargo-audit"),
            Self::Deny => Some("cargo-deny"),
            Self::Nextest => Some("cargo-nextest"),
            Self::LlvmCov => Some("cargo-llvm-cov"),
            _ => None,
        }
    }

    /// Returns the binary name to probe for availability.
    pub fn binary_name(&self) -> Option<&'static str> {
        match self {
            Self::Audit => Some("cargo-audit"),
            Self::Deny => Some("cargo-deny"),
            Self::Nextest => Some("cargo-nextest"),
            Self::LlvmCov => Some("cargo-llvm-cov"),
            _ => None,
        }
    }
}

/// Shared configuration for all cargo step types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CargoConfig {
    pub command: CargoCommand,
    /// Rust toolchain override (e.g. "nightly", "1.78.0").
    #[serde(default)]
    pub toolchain: Option<String>,
    /// Target package (`-p`).
    #[serde(default)]
    pub package: Option<String>,
    /// Features to enable (`--features`).
    #[serde(default)]
    pub features: Vec<String>,
    /// Enable all features (`--all-features`).
    #[serde(default)]
    pub all_features: bool,
    /// Disable default features (`--no-default-features`).
    #[serde(default)]
    pub no_default_features: bool,
    /// Build in release mode (`--release`).
    #[serde(default)]
    pub release: bool,
    /// Compilation target triple (`--target`).
    #[serde(default)]
    pub target: Option<String>,
    /// Build profile (`--profile`).
    #[serde(default)]
    pub profile: Option<String>,
    /// Additional arguments appended to the command.
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory.
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Execution timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Output directory for generated files (e.g., MDX docs).
    #[serde(default)]
    pub output_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn serde_round_trip_minimal() {
        let config = CargoConfig {
            command: CargoCommand::Build,
            toolchain: None,
            package: None,
            features: vec![],
            all_features: false,
            no_default_features: false,
            release: false,
            target: None,
            profile: None,
            extra_args: vec![],
            env: HashMap::new(),
            working_dir: None,
            timeout_ms: None,
            output_dir: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: CargoConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.command, CargoCommand::Build);
        assert!(de.features.is_empty());
        assert!(!de.release);
    }

    #[test]
    fn serde_round_trip_full() {
        let mut env = HashMap::new();
        env.insert("RUSTFLAGS".to_string(), "-D warnings".to_string());

        let config = CargoConfig {
            command: CargoCommand::Clippy,
            toolchain: Some("nightly".to_string()),
            package: Some("my-crate".to_string()),
            features: vec!["feat1".to_string(), "feat2".to_string()],
            all_features: false,
            no_default_features: true,
            release: true,
            target: Some("x86_64-unknown-linux-gnu".to_string()),
            profile: None,
            extra_args: vec!["--".to_string(), "-D".to_string(), "warnings".to_string()],
            env,
            working_dir: Some("/src".to_string()),
            timeout_ms: Some(60_000),
            output_dir: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let de: CargoConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.command, CargoCommand::Clippy);
        assert_eq!(de.toolchain, Some("nightly".to_string()));
        assert_eq!(de.package, Some("my-crate".to_string()));
        assert_eq!(de.features, vec!["feat1", "feat2"]);
        assert!(de.no_default_features);
        assert!(de.release);
        assert_eq!(de.extra_args, vec!["--", "-D", "warnings"]);
        assert_eq!(de.timeout_ms, Some(60_000));
    }

    #[test]
    fn command_as_str() {
        assert_eq!(CargoCommand::Build.as_str(), "build");
        assert_eq!(CargoCommand::Test.as_str(), "test");
        assert_eq!(CargoCommand::Check.as_str(), "check");
        assert_eq!(CargoCommand::Clippy.as_str(), "clippy");
        assert_eq!(CargoCommand::Fmt.as_str(), "fmt");
        assert_eq!(CargoCommand::Doc.as_str(), "doc");
        assert_eq!(CargoCommand::Publish.as_str(), "publish");
        assert_eq!(CargoCommand::Audit.as_str(), "audit");
        assert_eq!(CargoCommand::Deny.as_str(), "deny");
        assert_eq!(CargoCommand::Nextest.as_str(), "nextest");
        assert_eq!(CargoCommand::LlvmCov.as_str(), "llvm-cov");
        assert_eq!(CargoCommand::DocMdx.as_str(), "doc-mdx");
    }

    #[test]
    fn command_serde_kebab_case() {
        let json = r#""build""#;
        let cmd: CargoCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, CargoCommand::Build);

        let serialized = serde_json::to_string(&CargoCommand::Build).unwrap();
        assert_eq!(serialized, r#""build""#);

        // External tools
        let json = r#""llvm-cov""#;
        let cmd: CargoCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, CargoCommand::LlvmCov);

        let json = r#""nextest""#;
        let cmd: CargoCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, CargoCommand::Nextest);

        let json = r#""doc-mdx""#;
        let cmd: CargoCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd, CargoCommand::DocMdx);
    }

    #[test]
    fn subcommand_args_single() {
        assert_eq!(CargoCommand::Build.subcommand_args(), vec!["build"]);
        assert_eq!(CargoCommand::Audit.subcommand_args(), vec!["audit"]);
        assert_eq!(CargoCommand::LlvmCov.subcommand_args(), vec!["llvm-cov"]);
    }

    #[test]
    fn subcommand_args_nextest_has_run() {
        assert_eq!(
            CargoCommand::Nextest.subcommand_args(),
            vec!["nextest", "run"]
        );
    }

    #[test]
    fn subcommand_args_doc_mdx_uses_rustdoc() {
        assert_eq!(CargoCommand::DocMdx.subcommand_args(), vec!["rustdoc"]);
    }

    #[test]
    fn install_package_external_tools() {
        assert_eq!(CargoCommand::Audit.install_package(), Some("cargo-audit"));
        assert_eq!(CargoCommand::Deny.install_package(), Some("cargo-deny"));
        assert_eq!(
            CargoCommand::Nextest.install_package(),
            Some("cargo-nextest")
        );
        assert_eq!(
            CargoCommand::LlvmCov.install_package(),
            Some("cargo-llvm-cov")
        );
    }

    #[test]
    fn install_package_builtin_returns_none() {
        assert_eq!(CargoCommand::Build.install_package(), None);
        assert_eq!(CargoCommand::Test.install_package(), None);
        assert_eq!(CargoCommand::Check.install_package(), None);
        assert_eq!(CargoCommand::Clippy.install_package(), None);
        assert_eq!(CargoCommand::Fmt.install_package(), None);
        assert_eq!(CargoCommand::Doc.install_package(), None);
        assert_eq!(CargoCommand::Publish.install_package(), None);
        assert_eq!(CargoCommand::DocMdx.install_package(), None);
    }

    #[test]
    fn binary_name_external_tools() {
        assert_eq!(CargoCommand::Audit.binary_name(), Some("cargo-audit"));
        assert_eq!(CargoCommand::Deny.binary_name(), Some("cargo-deny"));
        assert_eq!(CargoCommand::Nextest.binary_name(), Some("cargo-nextest"));
        assert_eq!(CargoCommand::LlvmCov.binary_name(), Some("cargo-llvm-cov"));
    }

    #[test]
    fn binary_name_builtin_returns_none() {
        assert_eq!(CargoCommand::Build.binary_name(), None);
        assert_eq!(CargoCommand::Test.binary_name(), None);
    }

    #[test]
    fn config_defaults() {
        let json = r#"{"command": "test"}"#;
        let config: CargoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.command, CargoCommand::Test);
        assert!(config.toolchain.is_none());
        assert!(config.package.is_none());
        assert!(config.features.is_empty());
        assert!(!config.all_features);
        assert!(!config.no_default_features);
        assert!(!config.release);
        assert!(config.target.is_none());
        assert!(config.profile.is_none());
        assert!(config.extra_args.is_empty());
        assert!(config.env.is_empty());
        assert!(config.working_dir.is_none());
        assert!(config.timeout_ms.is_none());
        assert!(config.output_dir.is_none());
    }

    #[test]
    fn config_with_output_dir() {
        let json = r#"{"command": "doc-mdx", "output_dir": "docs/api"}"#;
        let config: CargoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.command, CargoCommand::DocMdx);
        assert_eq!(config.output_dir, Some("docs/api".to_string()));
    }
}
