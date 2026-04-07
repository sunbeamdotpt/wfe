//! Shared in-process gRPC stub server for command and integration tests.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::Mutex;
use tonic::transport::Server;
use tonic::{Request, Response, Status};

use wfe_server_protos::wfe::v1::{
    CancelWorkflowRequest, CancelWorkflowResponse, DefinitionSummary, GetWorkflowRequest,
    GetWorkflowResponse, ListDefinitionsRequest, ListDefinitionsResponse, PublishEventRequest,
    PublishEventResponse, RegisterWorkflowRequest, RegisterWorkflowResponse, RegisteredDefinition,
    ResumeWorkflowRequest, ResumeWorkflowResponse, SearchLogsRequest, SearchLogsResponse,
    SearchWorkflowsRequest, SearchWorkflowsResponse, StartWorkflowRequest, StartWorkflowResponse,
    SuspendWorkflowRequest, SuspendWorkflowResponse, WatchLifecycleRequest, WorkflowInstance,
    WorkflowSearchResult, WorkflowStatus,
    wfe_server::{Wfe, WfeServer},
};

#[derive(Default)]
pub struct StubWfe {
    pub seen_authorization: Arc<Mutex<Option<String>>>,
}

impl StubWfe {
    async fn capture_auth<T>(&self, req: &Request<T>) {
        if let Some(val) = req.metadata().get("authorization") {
            if let Ok(s) = val.to_str() {
                let mut guard = self.seen_authorization.lock().await;
                *guard = Some(s.to_string());
            }
        }
    }
}

#[tonic::async_trait]
impl Wfe for StubWfe {
    async fn register_workflow(
        &self,
        req: Request<RegisterWorkflowRequest>,
    ) -> Result<Response<RegisterWorkflowResponse>, Status> {
        self.capture_auth(&req).await;
        let inner = req.into_inner();
        Ok(Response::new(RegisterWorkflowResponse {
            definitions: vec![RegisteredDefinition {
                definition_id: format!("test-{}", inner.yaml.len()),
                version: 1,
                step_count: 3,
                name: "Test Workflow".into(),
            }],
        }))
    }

    async fn list_definitions(
        &self,
        req: Request<ListDefinitionsRequest>,
    ) -> Result<Response<ListDefinitionsResponse>, Status> {
        self.capture_auth(&req).await;
        Ok(Response::new(ListDefinitionsResponse {
            definitions: vec![DefinitionSummary {
                id: "ci".into(),
                version: 1,
                description: "CI pipeline".into(),
                step_count: 5,
                name: "Continuous Integration".into(),
            }],
        }))
    }

    async fn start_workflow(
        &self,
        req: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        self.capture_auth(&req).await;
        let inner = req.into_inner();
        let workflow_id = format!("wf-{}-{}", inner.definition_id, inner.version);
        let name = if inner.name.is_empty() {
            format!("{}-1", inner.definition_id)
        } else {
            inner.name
        };
        Ok(Response::new(StartWorkflowResponse { workflow_id, name }))
    }

    async fn get_workflow(
        &self,
        req: Request<GetWorkflowRequest>,
    ) -> Result<Response<GetWorkflowResponse>, Status> {
        self.capture_auth(&req).await;
        let id = req.into_inner().workflow_id;
        Ok(Response::new(GetWorkflowResponse {
            instance: Some(WorkflowInstance {
                id: id.clone(),
                name: "ci-1".into(),
                definition_id: "ci".into(),
                version: 1,
                description: "test instance".into(),
                reference: "ref-1".into(),
                status: WorkflowStatus::Runnable as i32,
                data: None,
                create_time: Some(prost_types::Timestamp {
                    seconds: 1_700_000_000,
                    nanos: 0,
                }),
                complete_time: None,
                execution_pointers: vec![wfe_server_protos::wfe::v1::ExecutionPointer {
                    id: "ptr-1".into(),
                    step_id: 0,
                    step_name: "build".into(),
                    status: wfe_server_protos::wfe::v1::PointerStatus::Complete as i32,
                    start_time: Some(prost_types::Timestamp {
                        seconds: 1_700_000_000,
                        nanos: 0,
                    }),
                    end_time: Some(prost_types::Timestamp {
                        seconds: 1_700_000_100,
                        nanos: 0,
                    }),
                    retry_count: 0,
                    active: false,
                }],
            }),
        }))
    }

    async fn cancel_workflow(
        &self,
        req: Request<CancelWorkflowRequest>,
    ) -> Result<Response<CancelWorkflowResponse>, Status> {
        self.capture_auth(&req).await;
        Ok(Response::new(CancelWorkflowResponse {}))
    }

    async fn suspend_workflow(
        &self,
        req: Request<SuspendWorkflowRequest>,
    ) -> Result<Response<SuspendWorkflowResponse>, Status> {
        self.capture_auth(&req).await;
        Ok(Response::new(SuspendWorkflowResponse {}))
    }

    async fn resume_workflow(
        &self,
        req: Request<ResumeWorkflowRequest>,
    ) -> Result<Response<ResumeWorkflowResponse>, Status> {
        self.capture_auth(&req).await;
        Ok(Response::new(ResumeWorkflowResponse {}))
    }

    async fn search_workflows(
        &self,
        req: Request<SearchWorkflowsRequest>,
    ) -> Result<Response<SearchWorkflowsResponse>, Status> {
        self.capture_auth(&req).await;
        Ok(Response::new(SearchWorkflowsResponse {
            results: vec![WorkflowSearchResult {
                id: "wf-1".into(),
                name: "ci-1".into(),
                definition_id: "ci".into(),
                version: 1,
                status: WorkflowStatus::Complete as i32,
                reference: "ref-1".into(),
                description: "test".into(),
                create_time: Some(prost_types::Timestamp {
                    seconds: 1_700_000_000,
                    nanos: 0,
                }),
            }],
            total: 1,
        }))
    }

    async fn publish_event(
        &self,
        req: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        self.capture_auth(&req).await;
        let event = req.into_inner();
        Ok(Response::new(PublishEventResponse {
            event_id: format!("evt-{}-{}", event.event_name, event.event_key),
        }))
    }

    type WatchLifecycleStream = tokio_stream::wrappers::ReceiverStream<
        Result<wfe_server_protos::wfe::v1::LifecycleEvent, Status>,
    >;
    async fn watch_lifecycle(
        &self,
        req: Request<WatchLifecycleRequest>,
    ) -> Result<Response<Self::WatchLifecycleStream>, Status> {
        self.capture_auth(&req).await;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let _ = tx
            .send(Ok(wfe_server_protos::wfe::v1::LifecycleEvent {
                event_time: Some(prost_types::Timestamp {
                    seconds: 1_700_000_000,
                    nanos: 0,
                }),
                workflow_id: "wf-1".into(),
                definition_id: "ci".into(),
                version: 1,
                event_type: wfe_server_protos::wfe::v1::LifecycleEventType::Started as i32,
                step_id: 0,
                step_name: String::new(),
                error_message: String::new(),
            }))
            .await;
        let _ = tx
            .send(Ok(wfe_server_protos::wfe::v1::LifecycleEvent {
                event_time: Some(prost_types::Timestamp {
                    seconds: 1_700_000_001,
                    nanos: 0,
                }),
                workflow_id: "wf-1".into(),
                definition_id: "ci".into(),
                version: 1,
                event_type: wfe_server_protos::wfe::v1::LifecycleEventType::StepCompleted as i32,
                step_id: 1,
                step_name: "build".into(),
                error_message: String::new(),
            }))
            .await;
        drop(tx);
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    type StreamLogsStream = tokio_stream::wrappers::ReceiverStream<
        Result<wfe_server_protos::wfe::v1::LogEntry, Status>,
    >;
    async fn stream_logs(
        &self,
        req: Request<wfe_server_protos::wfe::v1::StreamLogsRequest>,
    ) -> Result<Response<Self::StreamLogsStream>, Status> {
        self.capture_auth(&req).await;
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        let _ = tx
            .send(Ok(wfe_server_protos::wfe::v1::LogEntry {
                workflow_id: "wf-1".into(),
                step_name: "build".into(),
                step_id: 0,
                stream: wfe_server_protos::wfe::v1::LogStream::Stdout as i32,
                data: b"hello\n".to_vec(),
                timestamp: Some(prost_types::Timestamp {
                    seconds: 1_700_000_000,
                    nanos: 0,
                }),
            }))
            .await;
        let _ = tx
            .send(Ok(wfe_server_protos::wfe::v1::LogEntry {
                workflow_id: "wf-1".into(),
                step_name: "build".into(),
                step_id: 0,
                stream: wfe_server_protos::wfe::v1::LogStream::Stderr as i32,
                data: b"warning\n".to_vec(),
                timestamp: Some(prost_types::Timestamp {
                    seconds: 1_700_000_001,
                    nanos: 0,
                }),
            }))
            .await;
        drop(tx);
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    async fn search_logs(
        &self,
        req: Request<SearchLogsRequest>,
    ) -> Result<Response<SearchLogsResponse>, Status> {
        self.capture_auth(&req).await;
        let inner = req.into_inner();
        Ok(Response::new(SearchLogsResponse {
            results: vec![wfe_server_protos::wfe::v1::LogSearchResult {
                workflow_id: "wf-1".into(),
                definition_id: "ci".into(),
                step_name: "build".into(),
                line: format!("matched {}", inner.query),
                stream: wfe_server_protos::wfe::v1::LogStream::Stdout as i32,
                timestamp: Some(prost_types::Timestamp {
                    seconds: 1_700_000_000,
                    nanos: 0,
                }),
            }],
            total: 1,
        }))
    }
}

/// Spawn the stub server on an ephemeral port and return its URL + the
/// shared `seen_authorization` slot so tests can assert on it.
pub async fn spawn_stub() -> (String, Arc<Mutex<Option<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let stub = StubWfe::default();
    let seen = stub.seen_authorization.clone();

    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    tokio::spawn(async move {
        Server::builder()
            .add_service(WfeServer::new(stub))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });

    (format!("http://{addr}"), seen)
}
