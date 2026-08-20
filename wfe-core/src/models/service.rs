use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// An infrastructure service that runs alongside workflow steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDefinition {
    /// Service name -- used as DNS hostname (K8s) or env prefix (containerd).
    pub name: String,
    /// Container image to run.
    pub image: String,
    /// Ports exposed by the service.
    #[serde(default)]
    pub ports: Vec<ServicePort>,
    /// Environment variables for the service container.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// How to check if the service is ready to accept connections.
    #[serde(default)]
    pub readiness: Option<ReadinessProbe>,
    /// Override the container entrypoint.
    #[serde(default)]
    pub command: Vec<String>,
    /// Override the container command/args.
    #[serde(default)]
    pub args: Vec<String>,
    /// Memory limit (e.g., "512Mi").
    #[serde(default)]
    pub memory: Option<String>,
    /// CPU limit (e.g., "500m").
    #[serde(default)]
    pub cpu: Option<String>,
}

/// A port exposed by a service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServicePort {
    /// Container port.
    pub container_port: u16,
    #[serde(default)]
    /// Name.
    pub name: Option<String>,
    /// Protocol: "TCP" (default) or "UDP".
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

impl ServicePort {
    /// Create a TCP service port with no name.
    pub fn tcp(port: u16) -> Self {
        Self {
            container_port: port,
            name: None,
            protocol: "TCP".into(),
        }
    }
}

fn default_protocol() -> String {
    "TCP".into()
}

/// How to determine if a service is ready.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessProbe {
    /// Check.
    pub check: ReadinessCheck,
    /// Poll interval in milliseconds.
    #[serde(default = "default_5000")]
    pub interval_ms: u64,
    /// Total timeout in milliseconds.
    #[serde(default = "default_60000")]
    pub timeout_ms: u64,
    /// Maximum number of retries before giving up.
    #[serde(default = "default_12")]
    pub retries: u32,
}

/// The type of readiness check to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessCheck {
    /// Run a command inside the container.
    Exec(Vec<String>),
    /// Check if a TCP port is accepting connections.
    TcpSocket(u16),
    /// Make an HTTP GET request.
    HttpGet {
        /// Port to connect to.
        port: u16,
        /// URL path for the request.
        path: String,
    },
}

/// Runtime endpoint info for a provisioned service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceEndpoint {
    /// Name.
    pub name: String,
    /// Host.
    pub host: String,
    /// Ports.
    pub ports: Vec<ServicePort>,
}

fn default_5000() -> u64 {
    5000
}
fn default_60000() -> u64 {
    60000
}
fn default_12() -> u32 {
    12
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn service_definition_minimal_serde() {
        let json = r#"{"name":"postgres","image":"postgres:15"}"#;
        let svc: ServiceDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(svc.name, "postgres");
        assert_eq!(svc.image, "postgres:15");
        assert!(svc.ports.is_empty());
        assert!(svc.env.is_empty());
        assert!(svc.readiness.is_none());
        assert!(svc.command.is_empty());
        assert!(svc.memory.is_none());
    }

    #[test]
    fn service_definition_full_round_trip() {
        let svc = ServiceDefinition {
            name: "redis".into(),
            image: "redis:7-alpine".into(),
            ports: vec![ServicePort::tcp(6379)],
            env: [("REDIS_PASSWORD".into(), "secret".into())].into(),
            readiness: Some(ReadinessProbe {
                check: ReadinessCheck::TcpSocket(6379),
                interval_ms: 2000,
                timeout_ms: 30000,
                retries: 15,
            }),
            command: vec![],
            args: vec!["--requirepass".into(), "secret".into()],
            memory: Some("256Mi".into()),
            cpu: Some("250m".into()),
        };
        let json = serde_json::to_string(&svc).unwrap();
        let parsed: ServiceDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "redis");
        assert_eq!(parsed.ports.len(), 1);
        assert_eq!(parsed.ports[0].container_port, 6379);
        assert_eq!(parsed.args, vec!["--requirepass", "secret"]);
        assert_eq!(parsed.memory, Some("256Mi".into()));
    }

    #[test]
    fn service_port_tcp_helper() {
        let port = ServicePort::tcp(5432);
        assert_eq!(port.container_port, 5432);
        assert_eq!(port.protocol, "TCP");
        assert!(port.name.is_none());
    }

    #[test]
    fn service_port_default_protocol() {
        let json = r#"{"container_port": 8080}"#;
        let port: ServicePort = serde_json::from_str(json).unwrap();
        assert_eq!(port.protocol, "TCP");
    }

    #[test]
    fn readiness_probe_exec() {
        let probe = ReadinessProbe {
            check: ReadinessCheck::Exec(vec!["pg_isready".into(), "-U".into(), "postgres".into()]),
            interval_ms: 5000,
            timeout_ms: 60000,
            retries: 12,
        };
        let json = serde_json::to_string(&probe).unwrap();
        let parsed: ReadinessProbe = serde_json::from_str(&json).unwrap();
        match parsed.check {
            ReadinessCheck::Exec(cmd) => assert_eq!(cmd, vec!["pg_isready", "-U", "postgres"]),
            _ => panic!("expected Exec"),
        }
    }

    #[test]
    fn readiness_probe_tcp_socket() {
        let probe = ReadinessProbe {
            check: ReadinessCheck::TcpSocket(6379),
            interval_ms: 2000,
            timeout_ms: 30000,
            retries: 15,
        };
        let json = serde_json::to_string(&probe).unwrap();
        let parsed: ReadinessProbe = serde_json::from_str(&json).unwrap();
        match parsed.check {
            ReadinessCheck::TcpSocket(port) => assert_eq!(port, 6379),
            _ => panic!("expected TcpSocket"),
        }
    }

    #[test]
    fn readiness_probe_http_get() {
        let probe = ReadinessProbe {
            check: ReadinessCheck::HttpGet {
                port: 8080,
                path: "/health".into(),
            },
            interval_ms: 5000,
            timeout_ms: 60000,
            retries: 12,
        };
        let json = serde_json::to_string(&probe).unwrap();
        let parsed: ReadinessProbe = serde_json::from_str(&json).unwrap();
        match parsed.check {
            ReadinessCheck::HttpGet { port, path } => {
                assert_eq!(port, 8080);
                assert_eq!(path, "/health");
            }
            _ => panic!("expected HttpGet"),
        }
    }

    #[test]
    fn readiness_probe_defaults() {
        let json = r#"{"check": {"tcp_socket": 5432}}"#;
        let probe: ReadinessProbe = serde_json::from_str(json).unwrap();
        assert_eq!(probe.interval_ms, 5000);
        assert_eq!(probe.timeout_ms, 60000);
        assert_eq!(probe.retries, 12);
    }

    #[test]
    fn service_endpoint_serde() {
        let ep = ServiceEndpoint {
            name: "postgres".into(),
            host: "postgres.wfe-abc.svc.cluster.local".into(),
            ports: vec![ServicePort::tcp(5432)],
        };
        let json = serde_json::to_string(&ep).unwrap();
        let parsed: ServiceEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "postgres");
        assert_eq!(parsed.host, "postgres.wfe-abc.svc.cluster.local");
        assert_eq!(parsed.ports.len(), 1);
    }

    #[test]
    fn service_definition_with_env() {
        let svc = ServiceDefinition {
            name: "postgres".into(),
            image: "postgres:15".into(),
            ports: vec![ServicePort::tcp(5432)],
            env: [
                ("POSTGRES_PASSWORD".into(), "test".into()),
                ("POSTGRES_DB".into(), "myapp".into()),
            ]
            .into(),
            readiness: None,
            command: vec![],
            args: vec![],
            memory: None,
            cpu: None,
        };
        let json = serde_json::to_string(&svc).unwrap();
        let parsed: ServiceDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.env.get("POSTGRES_PASSWORD"), Some(&"test".into()));
        assert_eq!(parsed.env.get("POSTGRES_DB"), Some(&"myapp".into()));
    }

    #[test]
    fn service_definition_with_command() {
        let svc = ServiceDefinition {
            name: "custom".into(),
            image: "myimage:latest".into(),
            ports: vec![],
            env: HashMap::new(),
            readiness: None,
            command: vec!["/usr/bin/myserver".into()],
            args: vec!["--config".into(), "/etc/config.yaml".into()],
            memory: None,
            cpu: None,
        };
        let json = serde_json::to_string(&svc).unwrap();
        let parsed: ServiceDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.command, vec!["/usr/bin/myserver"]);
        assert_eq!(parsed.args, vec!["--config", "/etc/config.yaml"]);
    }
}
