use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use tonic::{Request, Response, Status};
use wfe_server_protos::wfe::v1::wfe_server::Wfe;
use wfe_server_protos::wfe::v1::*;

/// Wfeservice.
pub struct WfeService {
    host: Arc<wfe::WorkflowHost>,
    lifecycle_bus: Arc<crate::lifecycle_bus::BroadcastLifecyclePublisher>,
    log_store: Arc<crate::log_store::LogStore>,
    log_search: Option<Arc<crate::log_search::LogSearchIndex>>,
}

impl WfeService {
    pub fn new(
        host: Arc<wfe::WorkflowHost>,
        lifecycle_bus: Arc<crate::lifecycle_bus::BroadcastLifecyclePublisher>,
        log_store: Arc<crate::log_store::LogStore>,
    ) -> Self {
        Self {
            host,
            lifecycle_bus,
            log_store,
            log_search: None,
        }
    }

    pub fn with_log_search(mut self, index: Arc<crate::log_search::LogSearchIndex>) -> Self {
        self.log_search = Some(index);
        self
    }
}

#[tonic::async_trait]
impl Wfe for WfeService {
    // ── Definitions ──────────────────────────────────────────────────

    async fn register_workflow(
        &self,
        request: Request<RegisterWorkflowRequest>,
    ) -> Result<Response<RegisterWorkflowResponse>, Status> {
        let req = request.into_inner();

        let config: HashMap<String, serde_json::Value> = req
            .config
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect();

        let workflows = wfe_yaml::load_workflow_from_str(&req.yaml, &config)
            .map_err(|e| Status::invalid_argument(format!("YAML compilation failed: {e}")))?;

        let mut definitions = Vec::new();

        for compiled in workflows {
            for (key, factory) in compiled.step_factories {
                self.host.register_step_factory(&key, factory).await;
            }

            let id = compiled.definition.id.clone();
            let version = compiled.definition.version;
            let step_count = compiled.definition.steps.len() as u32;
            let name = compiled.definition.name.clone().unwrap_or_default();

            self.host
                .register_workflow_definition(compiled.definition)
                .await;

            definitions.push(RegisteredDefinition {
                definition_id: id,
                version,
                step_count,
                name,
            });
        }

        Ok(Response::new(RegisterWorkflowResponse { definitions }))
    }

    async fn list_definitions(
        &self,
        _request: Request<ListDefinitionsRequest>,
    ) -> Result<Response<ListDefinitionsResponse>, Status> {
        // TODO: add list_definitions() to WorkflowHost
        Ok(Response::new(ListDefinitionsResponse {
            definitions: vec![],
        }))
    }

    // ── Instances ────────────────────────────────────────────────────

    async fn start_workflow(
        &self,
        request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        let req = request.into_inner();

        let data = req
            .data
            .map(struct_to_json)
            .unwrap_or_else(|| serde_json::json!({}));

        // Empty `name` means "auto-assign"; pass None through so the host
        // generates `{definition_id}-{N}` via the persistence sequence.
        let name_override = if req.name.trim().is_empty() {
            None
        } else {
            Some(req.name)
        };

        let workflow_id = self
            .host
            .start_workflow_with_name(&req.definition_id, req.version, data, name_override)
            .await
            .map_err(|e| Status::internal(format!("failed to start workflow: {e}")))?;

        // Load the instance back so we can return the assigned name to the
        // client. Cheap read, single row, avoids plumbing the name through
        // the host's return signature.
        let instance = self
            .host
            .get_workflow(&workflow_id)
            .await
            .map_err(|e| Status::internal(format!("failed to load new workflow: {e}")))?;

        Ok(Response::new(StartWorkflowResponse {
            workflow_id,
            name: instance.name,
        }))
    }

    async fn get_workflow(
        &self,
        request: Request<GetWorkflowRequest>,
    ) -> Result<Response<GetWorkflowResponse>, Status> {
        let req = request.into_inner();

        let instance = self
            .host
            .get_workflow(&req.workflow_id)
            .await
            .map_err(|e| Status::not_found(format!("workflow not found: {e}")))?;

        Ok(Response::new(GetWorkflowResponse {
            instance: Some(workflow_to_proto(&instance)),
        }))
    }

    async fn cancel_workflow(
        &self,
        request: Request<CancelWorkflowRequest>,
    ) -> Result<Response<CancelWorkflowResponse>, Status> {
        let req = request.into_inner();

        self.host
            .terminate_workflow(&req.workflow_id)
            .await
            .map_err(|e| Status::internal(format!("failed to cancel: {e}")))?;

        Ok(Response::new(CancelWorkflowResponse {}))
    }

    async fn suspend_workflow(
        &self,
        request: Request<SuspendWorkflowRequest>,
    ) -> Result<Response<SuspendWorkflowResponse>, Status> {
        let req = request.into_inner();

        self.host
            .suspend_workflow(&req.workflow_id)
            .await
            .map_err(|e| Status::internal(format!("failed to suspend: {e}")))?;

        Ok(Response::new(SuspendWorkflowResponse {}))
    }

    async fn resume_workflow(
        &self,
        request: Request<ResumeWorkflowRequest>,
    ) -> Result<Response<ResumeWorkflowResponse>, Status> {
        let req = request.into_inner();

        self.host
            .resume_workflow(&req.workflow_id)
            .await
            .map_err(|e| Status::internal(format!("failed to resume: {e}")))?;

        Ok(Response::new(ResumeWorkflowResponse {}))
    }

    async fn search_workflows(
        &self,
        _request: Request<SearchWorkflowsRequest>,
    ) -> Result<Response<SearchWorkflowsResponse>, Status> {
        // TODO: implement with SearchIndex
        Ok(Response::new(SearchWorkflowsResponse {
            results: vec![],
            total: 0,
        }))
    }

    // ── Events ───────────────────────────────────────────────────────

    async fn publish_event(
        &self,
        request: Request<PublishEventRequest>,
    ) -> Result<Response<PublishEventResponse>, Status> {
        let req = request.into_inner();

        let data = req
            .data
            .map(struct_to_json)
            .unwrap_or_else(|| serde_json::json!({}));

        self.host
            .publish_event(&req.event_name, &req.event_key, data)
            .await
            .map_err(|e| Status::internal(format!("failed to publish event: {e}")))?;

        Ok(Response::new(PublishEventResponse {
            event_id: String::new(),
        }))
    }

    // ── Streaming (stubs for now) ────────────────────────────────────

    type WatchLifecycleStream =
        tokio_stream::wrappers::ReceiverStream<Result<LifecycleEvent, Status>>;

    async fn watch_lifecycle(
        &self,
        request: Request<WatchLifecycleRequest>,
    ) -> Result<Response<Self::WatchLifecycleStream>, Status> {
        let req = request.into_inner();
        // Resolve name-or-UUID to the canonical UUID upfront. Lifecycle events
        // carry UUIDs, so filtering by a human name would silently drop
        // everything. Empty filter means "all workflows".
        let filter_workflow_id = if req.workflow_id.is_empty() {
            None
        } else {
            let resolved = self
                .host
                .resolve_workflow_id(&req.workflow_id)
                .await
                .map_err(|e| Status::not_found(format!("workflow not found: {e}")))?;
            Some(resolved)
        };

        let mut broadcast_rx = self.lifecycle_bus.subscribe();
        let (tx, rx) = tokio::sync::mpsc::channel(256);

        tokio::spawn(async move {
            loop {
                match broadcast_rx.recv().await {
                    Ok(event) => {
                        // Apply workflow_id filter.
                        if let Some(ref filter) = filter_workflow_id {
                            if event.workflow_instance_id != *filter {
                                continue;
                            }
                        }
                        let proto_event = lifecycle_event_to_proto(&event);
                        if tx.send(Ok(proto_event)).await.is_err() {
                            break; // Client disconnected.
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(lagged = n, "lifecycle watcher lagged, skipping events");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    type StreamLogsStream = tokio_stream::wrappers::ReceiverStream<Result<LogEntry, Status>>;

    async fn stream_logs(
        &self,
        request: Request<StreamLogsRequest>,
    ) -> Result<Response<Self::StreamLogsStream>, Status> {
        let req = request.into_inner();
        // Resolve name-or-UUID so the log_store (which is keyed by UUID)
        // returns history for the right instance.
        let workflow_id = self
            .host
            .resolve_workflow_id(&req.workflow_id)
            .await
            .map_err(|e| Status::not_found(format!("workflow not found: {e}")))?;
        let step_name_filter = if req.step_name.is_empty() {
            None
        } else {
            Some(req.step_name)
        };

        let (tx, rx) = tokio::sync::mpsc::channel(256);
        let log_store = self.log_store.clone();

        tokio::spawn(async move {
            // 1. Replay history first.
            let history = log_store.get_history(&workflow_id, None);
            for chunk in history {
                if let Some(ref filter) = step_name_filter {
                    if chunk.step_name != *filter {
                        continue;
                    }
                }
                let entry = log_chunk_to_proto(&chunk);
                if tx.send(Ok(entry)).await.is_err() {
                    return; // Client disconnected.
                }
            }

            // 2. If follow mode, switch to live broadcast.
            if req.follow {
                let mut broadcast_rx = log_store.subscribe(&workflow_id);
                loop {
                    match broadcast_rx.recv().await {
                        Ok(chunk) => {
                            if let Some(ref filter) = step_name_filter {
                                if chunk.step_name != *filter {
                                    continue;
                                }
                            }
                            let entry = log_chunk_to_proto(&chunk);
                            if tx.send(Ok(entry)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(lagged = n, "log stream lagged");
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
            // If not follow mode, the stream ends after history replay.
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }

    // ── Search ───────────────────────────────────────────────────────

    async fn search_logs(
        &self,
        request: Request<SearchLogsRequest>,
    ) -> Result<Response<SearchLogsResponse>, Status> {
        let Some(ref search) = self.log_search else {
            return Err(Status::unavailable(
                "log search not configured — set --search-url",
            ));
        };

        let req = request.into_inner();
        // Resolve name-or-UUID upfront so the search index (keyed by UUID)
        // matches the requested instance. We materialize into a String so
        // the borrowed reference below has a stable lifetime.
        let resolved_workflow_id = if req.workflow_id.is_empty() {
            None
        } else {
            Some(
                self.host
                    .resolve_workflow_id(&req.workflow_id)
                    .await
                    .map_err(|e| Status::not_found(format!("workflow not found: {e}")))?,
            )
        };
        let workflow_id = resolved_workflow_id.as_deref();
        let step_name = if req.step_name.is_empty() {
            None
        } else {
            Some(req.step_name.as_str())
        };
        let stream_filter = match req.stream_filter {
            x if x == LogStream::Stdout as i32 => Some("stdout"),
            x if x == LogStream::Stderr as i32 => Some("stderr"),
            _ => None,
        };
        let take = if req.take == 0 { 50 } else { req.take };

        let (hits, total) = search
            .search(
                &req.query,
                workflow_id,
                step_name,
                stream_filter,
                req.skip,
                take,
            )
            .await
            .map_err(|e| Status::internal(format!("search failed: {e}")))?;

        let results = hits
            .into_iter()
            .map(|h| {
                let stream = match h.stream.as_str() {
                    "stdout" => LogStream::Stdout as i32,
                    "stderr" => LogStream::Stderr as i32,
                    _ => LogStream::Unspecified as i32,
                };
                LogSearchResult {
                    workflow_id: h.workflow_id,
                    definition_id: h.definition_id,
                    step_name: h.step_name,
                    line: h.line,
                    stream,
                    timestamp: Some(datetime_to_timestamp(&h.timestamp)),
                }
            })
            .collect();

        Ok(Response::new(SearchLogsResponse { results, total }))
    }
}

// ── Conversion helpers ──────────────────────────────────────────────

fn struct_to_json(s: prost_types::Struct) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = s
        .fields
        .into_iter()
        .map(|(k, v)| (k, prost_value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

fn prost_value_to_json(v: prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match v.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::NumberValue(n)) => serde_json::json!(n),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(b),
        Some(Kind::StructValue(s)) => struct_to_json(s),
        Some(Kind::ListValue(l)) => {
            serde_json::Value::Array(l.values.into_iter().map(prost_value_to_json).collect())
        }
        None => serde_json::Value::Null,
    }
}

fn json_to_struct(v: &serde_json::Value) -> prost_types::Struct {
    let fields: BTreeMap<String, prost_types::Value> = match v.as_object() {
        Some(obj) => obj
            .iter()
            .map(|(k, v)| (k.clone(), json_to_prost_value(v)))
            .collect(),
        None => BTreeMap::new(),
    };
    prost_types::Struct { fields }
}

fn json_to_prost_value(v: &serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => Kind::ListValue(prost_types::ListValue {
            values: arr.iter().map(json_to_prost_value).collect(),
        }),
        serde_json::Value::Object(_) => Kind::StructValue(json_to_struct(v)),
    };
    prost_types::Value { kind: Some(kind) }
}

fn log_chunk_to_proto(chunk: &wfe_core::traits::LogChunk) -> LogEntry {
    use wfe_core::traits::LogStreamType;
    let stream = match chunk.stream {
        LogStreamType::Stdout => LogStream::Stdout as i32,
        LogStreamType::Stderr => LogStream::Stderr as i32,
    };
    LogEntry {
        workflow_id: chunk.workflow_id.clone(),
        step_name: chunk.step_name.clone(),
        step_id: chunk.step_id as u32,
        stream,
        data: chunk.data.clone(),
        timestamp: Some(datetime_to_timestamp(&chunk.timestamp)),
    }
}

fn lifecycle_event_to_proto(e: &wfe_core::models::LifecycleEvent) -> LifecycleEvent {
    use wfe_core::models::LifecycleEventType as LET;
    // Proto enum — prost strips the LIFECYCLE_EVENT_TYPE_ prefix.
    use wfe_server_protos::wfe::v1::LifecycleEventType as PLET;
    let (event_type, step_id, step_name, error_message) = match &e.event_type {
        LET::Started => (PLET::Started as i32, 0, String::new(), String::new()),
        LET::Completed => (PLET::Completed as i32, 0, String::new(), String::new()),
        LET::Terminated => (PLET::Terminated as i32, 0, String::new(), String::new()),
        LET::Suspended => (PLET::Suspended as i32, 0, String::new(), String::new()),
        LET::Resumed => (PLET::Resumed as i32, 0, String::new(), String::new()),
        LET::Error { message } => (PLET::Error as i32, 0, String::new(), message.clone()),
        LET::StepStarted { step_id, step_name } => (
            PLET::StepStarted as i32,
            *step_id as u32,
            step_name.clone().unwrap_or_default(),
            String::new(),
        ),
        LET::StepCompleted { step_id, step_name } => (
            PLET::StepCompleted as i32,
            *step_id as u32,
            step_name.clone().unwrap_or_default(),
            String::new(),
        ),
    };
    LifecycleEvent {
        event_time: Some(datetime_to_timestamp(&e.event_time_utc)),
        workflow_id: e.workflow_instance_id.clone(),
        definition_id: e.workflow_definition_id.clone(),
        version: e.version,
        event_type,
        step_id,
        step_name,
        error_message,
    }
}

fn datetime_to_timestamp(dt: &chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

fn workflow_to_proto(w: &wfe_core::models::WorkflowInstance) -> WorkflowInstance {
    WorkflowInstance {
        id: w.id.clone(),
        name: w.name.clone(),
        definition_id: w.workflow_definition_id.clone(),
        version: w.version,
        description: w.description.clone().unwrap_or_default(),
        reference: w.reference.clone().unwrap_or_default(),
        status: match w.status {
            wfe_core::models::WorkflowStatus::Runnable => WorkflowStatus::Runnable as i32,
            wfe_core::models::WorkflowStatus::Suspended => WorkflowStatus::Suspended as i32,
            wfe_core::models::WorkflowStatus::Complete => WorkflowStatus::Complete as i32,
            wfe_core::models::WorkflowStatus::Terminated => WorkflowStatus::Terminated as i32,
        },
        data: Some(json_to_struct(&w.data)),
        create_time: Some(datetime_to_timestamp(&w.create_time)),
        complete_time: w.complete_time.as_ref().map(datetime_to_timestamp),
        execution_pointers: w.execution_pointers.iter().map(pointer_to_proto).collect(),
    }
}

fn pointer_to_proto(p: &wfe_core::models::ExecutionPointer) -> ExecutionPointer {
    use wfe_core::models::PointerStatus as PS;
    let status = match p.status {
        PS::Pending | PS::PendingPredecessor => PointerStatus::Pending as i32,
        PS::Running => PointerStatus::Running as i32,
        PS::Complete => PointerStatus::Complete as i32,
        PS::Sleeping => PointerStatus::Sleeping as i32,
        PS::WaitingForEvent => PointerStatus::WaitingForEvent as i32,
        PS::Failed => PointerStatus::Failed as i32,
        PS::Skipped => PointerStatus::Skipped as i32,
        PS::Compensated | PS::Cancelled => PointerStatus::Cancelled as i32,
    };
    ExecutionPointer {
        id: p.id.clone(),
        step_id: p.step_id as u32,
        step_name: p.step_name.clone().unwrap_or_default(),
        status,
        start_time: p.start_time.as_ref().map(datetime_to_timestamp),
        end_time: p.end_time.as_ref().map(datetime_to_timestamp),
        retry_count: p.retry_count,
        active: p.active,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_to_json_roundtrip() {
        let original = serde_json::json!({
            "name": "test",
            "count": 42.0,
            "active": true,
            "tags": ["a", "b"],
            "nested": { "key": "value" }
        });
        let proto_struct = json_to_struct(&original);
        let back = struct_to_json(proto_struct);
        assert_eq!(original, back);
    }

    #[test]
    fn json_null_roundtrip() {
        let v = serde_json::Value::Null;
        let pv = json_to_prost_value(&v);
        let back = prost_value_to_json(pv);
        assert_eq!(back, serde_json::Value::Null);
    }

    #[test]
    fn json_string_roundtrip() {
        let v = serde_json::Value::String("hello".to_string());
        let pv = json_to_prost_value(&v);
        let back = prost_value_to_json(pv);
        assert_eq!(back, v);
    }

    #[test]
    fn json_bool_roundtrip() {
        let v = serde_json::Value::Bool(true);
        let pv = json_to_prost_value(&v);
        let back = prost_value_to_json(pv);
        assert_eq!(back, v);
    }

    #[test]
    fn json_number_roundtrip() {
        let v = serde_json::json!(3.14);
        let pv = json_to_prost_value(&v);
        let back = prost_value_to_json(pv);
        assert_eq!(back, v);
    }

    #[test]
    fn json_array_roundtrip() {
        let v = serde_json::json!(["a", 1.0, true, null]);
        let pv = json_to_prost_value(&v);
        let back = prost_value_to_json(pv);
        assert_eq!(back, v);
    }

    #[test]
    fn empty_struct_roundtrip() {
        let v = serde_json::json!({});
        let proto_struct = json_to_struct(&v);
        let back = struct_to_json(proto_struct);
        assert_eq!(back, v);
    }

    #[test]
    fn prost_value_none_kind() {
        let v = prost_types::Value { kind: None };
        assert_eq!(prost_value_to_json(v), serde_json::Value::Null);
    }

    #[test]
    fn json_to_struct_from_non_object() {
        let v = serde_json::json!("not an object");
        let s = json_to_struct(&v);
        assert!(s.fields.is_empty());
    }

    #[test]
    fn datetime_to_timestamp_conversion() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-03-29T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let ts = datetime_to_timestamp(&dt);
        assert_eq!(ts.seconds, dt.timestamp());
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn workflow_status_mapping() {
        use wfe_core::models::{WorkflowInstance as WI, WorkflowStatus as WS};
        let mut w = WI::new("test", 1, serde_json::json!({}));

        w.status = WS::Runnable;
        let p = workflow_to_proto(&w);
        assert_eq!(p.status, WorkflowStatus::Runnable as i32);

        w.status = WS::Complete;
        let p = workflow_to_proto(&w);
        assert_eq!(p.status, WorkflowStatus::Complete as i32);

        w.status = WS::Suspended;
        let p = workflow_to_proto(&w);
        assert_eq!(p.status, WorkflowStatus::Suspended as i32);

        w.status = WS::Terminated;
        let p = workflow_to_proto(&w);
        assert_eq!(p.status, WorkflowStatus::Terminated as i32);
    }

    #[test]
    fn pointer_status_mapping() {
        use wfe_core::models::{ExecutionPointer as EP, PointerStatus as PS};
        let mut p = EP::new(0);

        p.status = PS::Pending;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Pending as i32);

        p.status = PS::Running;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Running as i32);

        p.status = PS::Complete;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Complete as i32);

        p.status = PS::Sleeping;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Sleeping as i32);

        p.status = PS::WaitingForEvent;
        assert_eq!(
            pointer_to_proto(&p).status,
            PointerStatus::WaitingForEvent as i32
        );

        p.status = PS::Failed;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Failed as i32);

        p.status = PS::Skipped;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Skipped as i32);

        p.status = PS::Cancelled;
        assert_eq!(pointer_to_proto(&p).status, PointerStatus::Cancelled as i32);
    }

    #[test]
    fn workflow_to_proto_basic() {
        let w =
            wfe_core::models::WorkflowInstance::new("my-wf", 1, serde_json::json!({"key": "val"}));
        let p = workflow_to_proto(&w);
        assert_eq!(p.definition_id, "my-wf");
        assert_eq!(p.version, 1);
        assert!(p.create_time.is_some());
        assert!(p.complete_time.is_none());
        let data = struct_to_json(p.data.unwrap());
        assert_eq!(data["key"], "val");
    }

    // ── gRPC integration tests with real WorkflowHost ────────────────

    async fn make_test_service() -> WfeService {
        use wfe::WorkflowHostBuilder;
        use wfe_core::test_support::{
            InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
        };

        let host = WorkflowHostBuilder::new()
            .use_persistence(std::sync::Arc::new(InMemoryPersistenceProvider::new())
                as std::sync::Arc<dyn wfe_core::traits::PersistenceProvider>)
            .use_lock_provider(std::sync::Arc::new(InMemoryLockProvider::new())
                as std::sync::Arc<dyn wfe_core::traits::DistributedLockProvider>)
            .use_queue_provider(std::sync::Arc::new(InMemoryQueueProvider::new())
                as std::sync::Arc<dyn wfe_core::traits::QueueProvider>)
            .build()
            .unwrap();

        host.start().await.unwrap();

        let lifecycle_bus =
            std::sync::Arc::new(crate::lifecycle_bus::BroadcastLifecyclePublisher::new(64));
        let log_store = std::sync::Arc::new(crate::log_store::LogStore::new());

        WfeService::new(std::sync::Arc::new(host), lifecycle_bus, log_store)
    }

    #[tokio::test]
    async fn rpc_register_and_start_workflow() {
        let svc = make_test_service().await;

        // Register a workflow.
        let req = Request::new(RegisterWorkflowRequest {
            yaml: r#"
workflow:
  id: test-wf
  version: 1
  steps:
    - name: hello
      type: shell
      config:
        run: echo hi
"#
            .to_string(),
            config: Default::default(),
        });
        let resp = svc.register_workflow(req).await.unwrap().into_inner();
        assert_eq!(resp.definitions.len(), 1);
        assert_eq!(resp.definitions[0].definition_id, "test-wf");
        assert_eq!(resp.definitions[0].version, 1);
        assert_eq!(resp.definitions[0].step_count, 1);

        // Start the workflow.
        let req = Request::new(StartWorkflowRequest {
            definition_id: "test-wf".to_string(),
            version: 1,
            data: None,
            name: String::new(),
        });
        let resp = svc.start_workflow(req).await.unwrap().into_inner();
        assert!(!resp.workflow_id.is_empty());

        // Get the workflow.
        let req = Request::new(GetWorkflowRequest {
            workflow_id: resp.workflow_id.clone(),
        });
        let resp = svc.get_workflow(req).await.unwrap().into_inner();
        let instance = resp.instance.unwrap();
        assert_eq!(instance.definition_id, "test-wf");
        assert_eq!(instance.status, WorkflowStatus::Runnable as i32);
    }

    #[tokio::test]
    async fn rpc_register_invalid_yaml() {
        let svc = make_test_service().await;
        let req = Request::new(RegisterWorkflowRequest {
            yaml: "not: valid: yaml: {{{}}}".to_string(),
            config: Default::default(),
        });
        let err = svc.register_workflow(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn rpc_start_nonexistent_workflow() {
        let svc = make_test_service().await;
        let req = Request::new(StartWorkflowRequest {
            definition_id: "nonexistent".to_string(),
            version: 1,
            data: None,
            name: String::new(),
        });
        let err = svc.start_workflow(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Internal);
    }

    #[tokio::test]
    async fn rpc_get_nonexistent_workflow() {
        let svc = make_test_service().await;
        let req = Request::new(GetWorkflowRequest {
            workflow_id: "nonexistent".to_string(),
        });
        let err = svc.get_workflow(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn rpc_cancel_workflow() {
        let svc = make_test_service().await;

        // Register + start.
        let req = Request::new(RegisterWorkflowRequest {
            yaml: "workflow:\n  id: cancel-test\n  version: 1\n  steps:\n    - name: s\n      type: shell\n      config:\n        run: echo ok\n".to_string(),
            config: Default::default(),
        });
        svc.register_workflow(req).await.unwrap();

        let req = Request::new(StartWorkflowRequest {
            definition_id: "cancel-test".to_string(),
            version: 1,
            data: None,
            name: String::new(),
        });
        let wf_id = svc
            .start_workflow(req)
            .await
            .unwrap()
            .into_inner()
            .workflow_id;

        // Cancel it.
        let req = Request::new(CancelWorkflowRequest {
            workflow_id: wf_id.clone(),
        });
        svc.cancel_workflow(req).await.unwrap();

        // Verify it's terminated.
        let req = Request::new(GetWorkflowRequest { workflow_id: wf_id });
        let instance = svc
            .get_workflow(req)
            .await
            .unwrap()
            .into_inner()
            .instance
            .unwrap();
        assert_eq!(instance.status, WorkflowStatus::Terminated as i32);
    }

    #[tokio::test]
    async fn rpc_suspend_resume_workflow() {
        let svc = make_test_service().await;

        let req = Request::new(RegisterWorkflowRequest {
            yaml: "workflow:\n  id: sr-test\n  version: 1\n  steps:\n    - name: s\n      type: shell\n      config:\n        run: echo ok\n".to_string(),
            config: Default::default(),
        });
        svc.register_workflow(req).await.unwrap();

        let req = Request::new(StartWorkflowRequest {
            definition_id: "sr-test".to_string(),
            version: 1,
            data: None,
            name: String::new(),
        });
        let wf_id = svc
            .start_workflow(req)
            .await
            .unwrap()
            .into_inner()
            .workflow_id;

        // Suspend.
        let req = Request::new(SuspendWorkflowRequest {
            workflow_id: wf_id.clone(),
        });
        svc.suspend_workflow(req).await.unwrap();

        let req = Request::new(GetWorkflowRequest {
            workflow_id: wf_id.clone(),
        });
        let instance = svc
            .get_workflow(req)
            .await
            .unwrap()
            .into_inner()
            .instance
            .unwrap();
        assert_eq!(instance.status, WorkflowStatus::Suspended as i32);

        // Resume.
        let req = Request::new(ResumeWorkflowRequest {
            workflow_id: wf_id.clone(),
        });
        svc.resume_workflow(req).await.unwrap();

        let req = Request::new(GetWorkflowRequest { workflow_id: wf_id });
        let instance = svc
            .get_workflow(req)
            .await
            .unwrap()
            .into_inner()
            .instance
            .unwrap();
        assert_eq!(instance.status, WorkflowStatus::Runnable as i32);
    }

    #[tokio::test]
    async fn rpc_publish_event() {
        let svc = make_test_service().await;
        let req = Request::new(PublishEventRequest {
            event_name: "test.event".to_string(),
            event_key: "key-1".to_string(),
            data: None,
        });
        // Should succeed even with no waiting workflows.
        svc.publish_event(req).await.unwrap();
    }

    #[tokio::test]
    async fn rpc_search_logs_not_configured() {
        let svc = make_test_service().await;
        let req = Request::new(SearchLogsRequest {
            query: "test".to_string(),
            ..Default::default()
        });
        let err = svc.search_logs(req).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unavailable);
    }

    #[tokio::test]
    async fn rpc_list_definitions_empty() {
        let svc = make_test_service().await;
        let req = Request::new(ListDefinitionsRequest {});
        let resp = svc.list_definitions(req).await.unwrap().into_inner();
        assert!(resp.definitions.is_empty());
    }

    #[tokio::test]
    async fn rpc_search_workflows_empty() {
        let svc = make_test_service().await;
        let req = Request::new(SearchWorkflowsRequest {
            query: "test".to_string(),
            ..Default::default()
        });
        let resp = svc.search_workflows(req).await.unwrap().into_inner();
        assert_eq!(resp.total, 0);
    }
}
