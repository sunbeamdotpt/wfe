use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{
    Container, ContainerPort, EnvVar, Pod, PodSpec, ResourceRequirements, Service, ServicePort,
    ServiceSpec,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::ObjectMeta;
use wfe_core::models::service::ServiceDefinition;

const LABEL_SERVICE_NAME: &str = "wfe.sunbeam.pt/service-name";
const LABEL_MANAGED_BY: &str = "wfe.sunbeam.pt/managed-by";

/// Build a Kubernetes Pod for a service definition.
pub fn build_service_pod(svc: &ServiceDefinition, namespace: &str) -> Pod {
    let mut labels = BTreeMap::new();
    labels.insert(LABEL_SERVICE_NAME.into(), svc.name.clone());
    labels.insert(LABEL_MANAGED_BY.into(), "wfe-kubernetes".into());

    let ports: Vec<ContainerPort> = svc
        .ports
        .iter()
        .map(|p| ContainerPort {
            container_port: p.container_port as i32,
            name: p.name.clone(),
            protocol: Some(p.protocol.clone()),
            ..Default::default()
        })
        .collect();

    let env: Vec<EnvVar> = svc
        .env
        .iter()
        .map(|(k, v)| EnvVar {
            name: k.clone(),
            value: Some(v.clone()),
            ..Default::default()
        })
        .collect();

    let mut limits = BTreeMap::new();
    let mut requests = BTreeMap::new();
    if let Some(ref mem) = svc.memory {
        limits.insert("memory".into(), Quantity(mem.clone()));
        requests.insert("memory".into(), Quantity(mem.clone()));
    }
    if let Some(ref cpu) = svc.cpu {
        limits.insert("cpu".into(), Quantity(cpu.clone()));
        requests.insert("cpu".into(), Quantity(cpu.clone()));
    }

    let command = if svc.command.is_empty() {
        None
    } else {
        Some(svc.command.clone())
    };
    let args = if svc.args.is_empty() {
        None
    } else {
        Some(svc.args.clone())
    };

    Pod {
        metadata: ObjectMeta {
            name: Some(svc.name.clone()),
            namespace: Some(namespace.into()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        spec: Some(PodSpec {
            containers: vec![Container {
                name: svc.name.clone(),
                image: Some(svc.image.clone()),
                ports: Some(ports),
                env: Some(env),
                command,
                args,
                resources: Some(ResourceRequirements {
                    limits: if limits.is_empty() {
                        None
                    } else {
                        Some(limits)
                    },
                    requests: if requests.is_empty() {
                        None
                    } else {
                        Some(requests)
                    },
                    ..Default::default()
                }),
                ..Default::default()
            }],
            restart_policy: Some("Always".into()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Build a Kubernetes Service for DNS resolution of a service definition.
pub fn build_k8s_service(svc: &ServiceDefinition, namespace: &str) -> Service {
    let mut selector = BTreeMap::new();
    selector.insert(LABEL_SERVICE_NAME.into(), svc.name.clone());

    let ports: Vec<ServicePort> = svc
        .ports
        .iter()
        .enumerate()
        .map(|(i, p)| ServicePort {
            name: p.name.clone().or_else(|| Some(format!("port-{i}"))),
            port: p.container_port as i32,
            target_port: Some(IntOrString::Int(p.container_port as i32)),
            protocol: Some(p.protocol.clone()),
            ..Default::default()
        })
        .collect();

    Service {
        metadata: ObjectMeta {
            name: Some(svc.name.clone()),
            namespace: Some(namespace.into()),
            labels: Some({
                let mut labels = BTreeMap::new();
                labels.insert(LABEL_MANAGED_BY.into(), "wfe-kubernetes".into());
                labels
            }),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(selector),
            ports: Some(ports),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use wfe_core::models::service::ServicePort as WfeServicePort;

    fn postgres_service() -> ServiceDefinition {
        ServiceDefinition {
            name: "postgres".into(),
            image: "postgres:15".into(),
            ports: vec![WfeServicePort::tcp(5432)],
            env: [
                ("POSTGRES_PASSWORD".into(), "test".into()),
                ("POSTGRES_DB".into(), "myapp".into()),
            ]
            .into(),
            readiness: None,
            command: vec![],
            args: vec![],
            memory: Some("512Mi".into()),
            cpu: Some("500m".into()),
        }
    }

    #[test]
    fn build_pod_basic() {
        let svc = postgres_service();
        let pod = build_service_pod(&svc, "wfe-test");

        assert_eq!(pod.metadata.name, Some("postgres".into()));
        assert_eq!(pod.metadata.namespace, Some("wfe-test".into()));

        let spec = pod.spec.as_ref().unwrap();
        assert_eq!(spec.restart_policy, Some("Always".into()));
        assert_eq!(spec.containers.len(), 1);

        let container = &spec.containers[0];
        assert_eq!(container.name, "postgres");
        assert_eq!(container.image, Some("postgres:15".into()));

        let ports = container.ports.as_ref().unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].container_port, 5432);

        let env = container.env.as_ref().unwrap();
        assert!(env.iter().any(|e| e.name == "POSTGRES_PASSWORD"));
    }

    #[test]
    fn build_pod_with_resources() {
        let svc = postgres_service();
        let pod = build_service_pod(&svc, "ns");
        let container = &pod.spec.unwrap().containers[0];
        let resources = container.resources.as_ref().unwrap();
        let limits = resources.limits.as_ref().unwrap();
        assert_eq!(limits.get("memory"), Some(&Quantity("512Mi".into())));
        assert_eq!(limits.get("cpu"), Some(&Quantity("500m".into())));
    }

    #[test]
    fn build_pod_labels() {
        let svc = postgres_service();
        let pod = build_service_pod(&svc, "ns");
        let labels = pod.metadata.labels.as_ref().unwrap();
        assert_eq!(labels.get(LABEL_SERVICE_NAME), Some(&"postgres".into()));
        assert_eq!(labels.get(LABEL_MANAGED_BY), Some(&"wfe-kubernetes".into()));
    }

    #[test]
    fn build_pod_with_command() {
        let svc = ServiceDefinition {
            name: "custom".into(),
            image: "myimage".into(),
            ports: vec![],
            env: Default::default(),
            readiness: None,
            command: vec!["/usr/bin/server".into()],
            args: vec!["--port".into(), "8080".into()],
            memory: None,
            cpu: None,
        };
        let pod = build_service_pod(&svc, "ns");
        let container = &pod.spec.unwrap().containers[0];
        assert_eq!(container.command, Some(vec!["/usr/bin/server".into()]));
        assert_eq!(container.args, Some(vec!["--port".into(), "8080".into()]));
    }

    #[test]
    fn build_k8s_service_basic() {
        let svc = postgres_service();
        let k8s_svc = build_k8s_service(&svc, "wfe-test");

        assert_eq!(k8s_svc.metadata.name, Some("postgres".into()));
        assert_eq!(k8s_svc.metadata.namespace, Some("wfe-test".into()));

        let spec = k8s_svc.spec.as_ref().unwrap();
        let selector = spec.selector.as_ref().unwrap();
        assert_eq!(selector.get(LABEL_SERVICE_NAME), Some(&"postgres".into()));

        let ports = spec.ports.as_ref().unwrap();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].port, 5432);
        assert_eq!(ports[0].target_port, Some(IntOrString::Int(5432)));
    }

    #[test]
    fn build_k8s_service_multiple_ports() {
        let svc = ServiceDefinition {
            name: "app".into(),
            image: "myapp".into(),
            ports: vec![WfeServicePort::tcp(8080), WfeServicePort::tcp(8443)],
            env: Default::default(),
            readiness: None,
            command: vec![],
            args: vec![],
            memory: None,
            cpu: None,
        };
        let k8s_svc = build_k8s_service(&svc, "ns");
        let ports = k8s_svc.spec.unwrap().ports.unwrap();
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].port, 8080);
        assert_eq!(ports[1].port, 8443);
    }
}
