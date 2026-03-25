use std::fmt;
use std::path::Path;

use super::config::DenoPermissions;

/// Error returned when a permission check fails.
#[derive(Debug, Clone)]
pub struct PermissionError {
    pub kind: &'static str,
    pub resource: String,
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Permission denied: {} access to '{}'",
            self.kind, self.resource
        )
    }
}

impl std::error::Error for PermissionError {}

/// Checks whether operations are allowed based on the configured permissions.
pub struct PermissionChecker {
    net_hosts: Vec<String>,
    read_paths: Vec<String>,
    write_paths: Vec<String>,
    env_vars: Vec<String>,
    pub allow_run: bool,
    pub allow_dynamic_import: bool,
}

impl PermissionChecker {
    /// Build a checker from the YAML permission config.
    pub fn from_config(perms: &DenoPermissions) -> Self {
        Self {
            net_hosts: perms.net.clone(),
            read_paths: perms.read.clone(),
            write_paths: perms.write.clone(),
            env_vars: perms.env.clone(),
            allow_run: perms.run,
            allow_dynamic_import: perms.dynamic_import,
        }
    }

    /// Check whether network access to `host` is allowed.
    pub fn check_net(&self, host: &str) -> Result<(), PermissionError> {
        if self.net_hosts.iter().any(|h| h == host) {
            Ok(())
        } else {
            Err(PermissionError {
                kind: "net",
                resource: host.to_string(),
            })
        }
    }

    /// Check whether file-system read access to `path` is allowed.
    pub fn check_read(&self, path: &str) -> Result<(), PermissionError> {
        // Block path traversal attempts.
        if Self::has_traversal(path) {
            return Err(PermissionError {
                kind: "read",
                resource: path.to_string(),
            });
        }

        let target = Path::new(path);
        if self
            .read_paths
            .iter()
            .any(|allowed| target.starts_with(allowed))
        {
            Ok(())
        } else {
            Err(PermissionError {
                kind: "read",
                resource: path.to_string(),
            })
        }
    }

    /// Check whether file-system write access to `path` is allowed.
    pub fn check_write(&self, path: &str) -> Result<(), PermissionError> {
        if Self::has_traversal(path) {
            return Err(PermissionError {
                kind: "write",
                resource: path.to_string(),
            });
        }

        let target = Path::new(path);
        if self
            .write_paths
            .iter()
            .any(|allowed| target.starts_with(allowed))
        {
            Ok(())
        } else {
            Err(PermissionError {
                kind: "write",
                resource: path.to_string(),
            })
        }
    }

    /// Check whether reading environment variable `var` is allowed.
    pub fn check_env(&self, var: &str) -> Result<(), PermissionError> {
        if self.env_vars.iter().any(|v| v == var) {
            Ok(())
        } else {
            Err(PermissionError {
                kind: "env",
                resource: var.to_string(),
            })
        }
    }

    /// Detect `..` path traversal components.
    fn has_traversal(path: &str) -> bool {
        Path::new(path).components().any(|c| {
            matches!(c, std::path::Component::ParentDir)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(
        net: &[&str],
        read: &[&str],
        write: &[&str],
        env: &[&str],
    ) -> PermissionChecker {
        PermissionChecker::from_config(&DenoPermissions {
            net: net.iter().map(|s| s.to_string()).collect(),
            read: read.iter().map(|s| s.to_string()).collect(),
            write: write.iter().map(|s| s.to_string()).collect(),
            env: env.iter().map(|s| s.to_string()).collect(),
            run: false,
            dynamic_import: false,
        })
    }

    #[test]
    fn net_allowed_host_passes() {
        let checker = perms(&["api.example.com"], &[], &[], &[]);
        assert!(checker.check_net("api.example.com").is_ok());
    }

    #[test]
    fn net_denied_host_fails() {
        let checker = perms(&["api.example.com"], &[], &[], &[]);
        let err = checker.check_net("evil.com").unwrap_err();
        assert_eq!(err.kind, "net");
        assert_eq!(err.resource, "evil.com");
    }

    #[test]
    fn net_empty_allowlist_denies_all() {
        let checker = perms(&[], &[], &[], &[]);
        assert!(checker.check_net("anything.com").is_err());
    }

    #[test]
    fn read_allowed_path_passes() {
        let checker = perms(&[], &["/tmp"], &[], &[]);
        assert!(checker.check_read("/tmp/data.json").is_ok());
    }

    #[test]
    fn read_denied_path_fails() {
        let checker = perms(&[], &["/tmp"], &[], &[]);
        let err = checker.check_read("/etc/passwd").unwrap_err();
        assert_eq!(err.kind, "read");
    }

    #[test]
    fn read_path_traversal_blocked() {
        let checker = perms(&[], &["/tmp"], &[], &[]);
        let err = checker
            .check_read("/tmp/../../../etc/passwd")
            .unwrap_err();
        assert_eq!(err.kind, "read");
        assert!(err.resource.contains(".."));
    }

    #[test]
    fn write_allowed_path_passes() {
        let checker = perms(&[], &[], &["/tmp/out"], &[]);
        assert!(checker.check_write("/tmp/out/result.json").is_ok());
    }

    #[test]
    fn write_denied_path_fails() {
        let checker = perms(&[], &[], &["/tmp/out"], &[]);
        let err = checker.check_write("/var/data").unwrap_err();
        assert_eq!(err.kind, "write");
    }

    #[test]
    fn write_path_traversal_blocked() {
        let checker = perms(&[], &[], &["/tmp/out"], &[]);
        assert!(checker
            .check_write("/tmp/out/../../etc/shadow")
            .is_err());
    }

    #[test]
    fn env_allowed_var_passes() {
        let checker = perms(&[], &[], &[], &["HOME", "PATH"]);
        assert!(checker.check_env("HOME").is_ok());
    }

    #[test]
    fn env_denied_var_fails() {
        let checker = perms(&[], &[], &[], &["HOME"]);
        let err = checker.check_env("SECRET_KEY").unwrap_err();
        assert_eq!(err.kind, "env");
        assert_eq!(err.resource, "SECRET_KEY");
    }

    #[test]
    fn permission_error_display() {
        let err = PermissionError {
            kind: "net",
            resource: "evil.com".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Permission denied: net access to 'evil.com'"
        );
    }

    #[test]
    fn from_config_copies_all_fields() {
        let config = DenoPermissions {
            net: vec!["a.com".to_string()],
            read: vec!["/r".to_string()],
            write: vec!["/w".to_string()],
            env: vec!["E".to_string()],
            run: true,
            dynamic_import: true,
        };
        let checker = PermissionChecker::from_config(&config);
        assert!(checker.allow_run);
        assert!(checker.allow_dynamic_import);
        assert!(checker.check_net("a.com").is_ok());
        assert!(checker.check_read("/r/file").is_ok());
        assert!(checker.check_write("/w/file").is_ok());
        assert!(checker.check_env("E").is_ok());
    }
}
