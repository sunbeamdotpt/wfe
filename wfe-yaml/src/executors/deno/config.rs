use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Configuration for a Deno JavaScript/TypeScript step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DenoConfig {
    /// Inline script source code.
    pub script: Option<String>,
    /// Path to a script file.
    pub file: Option<String>,
    /// Permission allowlists.
    #[serde(default)]
    pub permissions: DenoPermissions,
    /// ES modules to pre-load.
    #[serde(default)]
    pub modules: Vec<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Execution timeout in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// Fine-grained permission allowlists for the Deno sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DenoPermissions {
    /// Allowed network hosts (e.g. `["api.example.com"]`).
    #[serde(default)]
    pub net: Vec<String>,
    /// Allowed file-system read paths.
    #[serde(default)]
    pub read: Vec<String>,
    /// Allowed file-system write paths.
    #[serde(default)]
    pub write: Vec<String>,
    /// Allowed environment variable names.
    #[serde(default)]
    pub env: Vec<String>,
    /// Whether sub-process spawning is allowed.
    #[serde(default)]
    pub run: bool,
    /// Whether dynamic `import()` is allowed.
    #[serde(default)]
    pub dynamic_import: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_full_config() {
        let config = DenoConfig {
            script: Some("console.log('hi')".to_string()),
            file: None,
            permissions: DenoPermissions {
                net: vec!["example.com".to_string()],
                read: vec!["/tmp".to_string()],
                write: vec!["/tmp/out".to_string()],
                env: vec!["HOME".to_string()],
                run: false,
                dynamic_import: true,
            },
            modules: vec!["https://deno.land/std/mod.ts".to_string()],
            env: {
                let mut m = HashMap::new();
                m.insert("FOO".to_string(), "bar".to_string());
                m
            },
            timeout_ms: Some(5000),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: DenoConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.script.as_deref(), Some("console.log('hi')"));
        assert!(deserialized.file.is_none());
        assert_eq!(deserialized.permissions.net, vec!["example.com"]);
        assert_eq!(deserialized.permissions.read, vec!["/tmp"]);
        assert_eq!(deserialized.permissions.write, vec!["/tmp/out"]);
        assert_eq!(deserialized.permissions.env, vec!["HOME"]);
        assert!(!deserialized.permissions.run);
        assert!(deserialized.permissions.dynamic_import);
        assert_eq!(deserialized.modules.len(), 1);
        assert_eq!(deserialized.env.get("FOO").unwrap(), "bar");
        assert_eq!(deserialized.timeout_ms, Some(5000));
    }

    #[test]
    fn serde_round_trip_defaults() {
        let json = r#"{"script": "1+1"}"#;
        let config: DenoConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.script.as_deref(), Some("1+1"));
        assert!(config.file.is_none());
        assert!(config.permissions.net.is_empty());
        assert!(config.permissions.read.is_empty());
        assert!(config.permissions.write.is_empty());
        assert!(config.permissions.env.is_empty());
        assert!(!config.permissions.run);
        assert!(!config.permissions.dynamic_import);
        assert!(config.modules.is_empty());
        assert!(config.env.is_empty());
        assert!(config.timeout_ms.is_none());
    }

    #[test]
    fn serde_round_trip_file_config() {
        let config = DenoConfig {
            script: None,
            file: Some("./scripts/run.ts".to_string()),
            permissions: DenoPermissions::default(),
            modules: vec![],
            env: HashMap::new(),
            timeout_ms: Some(10000),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: DenoConfig = serde_json::from_str(&json).unwrap();
        assert!(deserialized.script.is_none());
        assert_eq!(deserialized.file.as_deref(), Some("./scripts/run.ts"));
        assert_eq!(deserialized.timeout_ms, Some(10000));
    }
}
