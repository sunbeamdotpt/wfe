use futures::StreamExt;
use futures::io::AsyncBufReadExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::LogParams;
use kube::{Api, Client};
use wfe_core::WfeError;
use wfe_core::traits::log_sink::{LogChunk, LogSink, LogStreamType};

/// Stream logs from a pod container, optionally forwarding to a LogSink.
///
/// Returns the full stdout content for output parsing.
/// Blocks until the container terminates and all logs are consumed.
pub async fn stream_logs(
    client: &Client,
    namespace: &str,
    pod_name: &str,
    step_name: &str,
    definition_id: &str,
    workflow_id: &str,
    step_id: usize,
    log_sink: Option<&dyn LogSink>,
) -> Result<String, WfeError> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    let params = LogParams {
        follow: true,
        container: Some("step".into()),
        ..Default::default()
    };

    let stream = pods.log_stream(pod_name, &params).await.map_err(|e| {
        WfeError::StepExecution(format!("failed to stream logs from pod '{pod_name}': {e}"))
    })?;

    let mut stdout = String::new();
    let reader = futures::io::BufReader::new(stream);
    let mut lines = reader.lines();

    while let Some(line_result) = lines.next().await {
        let line: String = line_result.map_err(|e| {
            WfeError::StepExecution(format!("log stream error for pod '{pod_name}': {e}"))
        })?;
        stdout.push_str(&line);
        stdout.push('\n');

        if let Some(sink) = log_sink {
            let mut data = line.into_bytes();
            data.push(b'\n');
            sink.write_chunk(LogChunk {
                workflow_id: workflow_id.to_string(),
                definition_id: definition_id.to_string(),
                step_id,
                step_name: step_name.to_string(),
                stream: LogStreamType::Stdout,
                data,
                timestamp: chrono::Utc::now(),
            })
            .await;
        }
    }

    Ok(stdout)
}

/// Wait for a pod's container to be in a running or terminated state.
pub async fn wait_for_pod_running(
    client: &Client,
    namespace: &str,
    pod_name: &str,
) -> Result<(), WfeError> {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);

    for _ in 0..120 {
        match pods.get(pod_name).await {
            Ok(pod) => {
                if let Some(status) = &pod.status {
                    if let Some(container_statuses) = &status.container_statuses {
                        for cs in container_statuses {
                            if let Some(state) = &cs.state {
                                if state.running.is_some() || state.terminated.is_some() {
                                    return Ok(());
                                }
                            }
                        }
                    }
                    if let Some(conditions) = &status.conditions {
                        for cond in conditions {
                            if cond.type_ == "PodScheduled" && cond.status == "False" {
                                if let Some(ref msg) = cond.message {
                                    return Err(WfeError::StepExecution(format!(
                                        "pod '{pod_name}' scheduling failed: {msg}"
                                    )));
                                }
                            }
                        }
                    }
                }
            }
            Err(kube::Error::Api(err)) if err.code == 404 => {}
            Err(e) => {
                return Err(WfeError::StepExecution(format!(
                    "failed to get pod '{pod_name}': {e}"
                )));
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Err(WfeError::StepExecution(format!(
        "pod '{pod_name}' did not start within 120s"
    )))
}
