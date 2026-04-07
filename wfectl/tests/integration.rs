//! Raw gRPC client tests against the in-process stub server.
//! Verifies the wire protocol and bearer auth interceptor.

mod stub;

use wfe_server_protos::wfe::v1::{
    CancelWorkflowRequest, GetWorkflowRequest, ListDefinitionsRequest, PublishEventRequest,
    RegisterWorkflowRequest, ResumeWorkflowRequest, SearchLogsRequest, SearchWorkflowsRequest,
    StartWorkflowRequest, SuspendWorkflowRequest, WatchLifecycleRequest, WorkflowStatus,
};

use wfectl::client::build as build_client;

use stub::spawn_stub;

#[tokio::test]
async fn client_register_workflow() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .register_workflow(RegisterWorkflowRequest {
            yaml: "workflow:\n  id: x\n  version: 1\n  steps: []".into(),
            config: Default::default(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.definitions.len(), 1);
    assert_eq!(resp.definitions[0].step_count, 3);
}

#[tokio::test]
async fn client_list_definitions() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .list_definitions(ListDefinitionsRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.definitions.len(), 1);
    assert_eq!(resp.definitions[0].id, "ci");
}

#[tokio::test]
async fn client_start_workflow_with_data() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .start_workflow(StartWorkflowRequest {
            definition_id: "ci".into(),
            version: 1,
            data: Some(wfectl::struct_util::json_object_to_struct(
                &serde_json::json!({"key": "value"}),
            )),
            name: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.workflow_id, "wf-ci-1");
}

#[tokio::test]
async fn client_get_workflow_returns_instance() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .get_workflow(GetWorkflowRequest {
            workflow_id: "wf-ci-1".into(),
        })
        .await
        .unwrap()
        .into_inner();
    let instance = resp.instance.unwrap();
    assert_eq!(instance.id, "wf-ci-1");
    assert_eq!(instance.definition_id, "ci");
}

#[tokio::test]
async fn client_cancel_suspend_resume() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    client
        .cancel_workflow(CancelWorkflowRequest {
            workflow_id: "wf-1".into(),
        })
        .await
        .unwrap();
    client
        .suspend_workflow(SuspendWorkflowRequest {
            workflow_id: "wf-1".into(),
        })
        .await
        .unwrap();
    client
        .resume_workflow(ResumeWorkflowRequest {
            workflow_id: "wf-1".into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn client_search_workflows() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .search_workflows(SearchWorkflowsRequest {
            query: "ci".into(),
            status_filter: WorkflowStatus::Complete as i32,
            skip: 0,
            take: 10,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.total, 1);
    assert_eq!(resp.results[0].id, "wf-1");
}

#[tokio::test]
async fn client_publish_event() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .publish_event(PublishEventRequest {
            event_name: "order.paid".into(),
            event_key: "order-42".into(),
            data: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.event_id, "evt-order.paid-order-42");
}

#[tokio::test]
async fn client_watch_lifecycle_stream() {
    use futures::StreamExt;
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let mut stream = client
        .watch_lifecycle(WatchLifecycleRequest {
            workflow_id: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    let mut count = 0;
    while let Some(event) = stream.next().await {
        let event = event.unwrap();
        assert_eq!(event.workflow_id, "wf-1");
        count += 1;
    }
    assert_eq!(count, 2);
}

#[tokio::test]
async fn client_stream_logs() {
    use futures::StreamExt;
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let mut stream = client
        .stream_logs(wfe_server_protos::wfe::v1::StreamLogsRequest {
            workflow_id: "wf-1".into(),
            step_name: String::new(),
            follow: false,
        })
        .await
        .unwrap()
        .into_inner();
    let entry = stream.next().await.unwrap().unwrap();
    assert_eq!(entry.step_name, "build");
    assert_eq!(entry.data, b"hello\n");
}

#[tokio::test]
async fn client_search_logs() {
    let (server, _seen) = spawn_stub().await;
    let mut client = build_client(&server, "test-token").await.unwrap();
    let resp = client
        .search_logs(SearchLogsRequest {
            query: "needle".into(),
            workflow_id: String::new(),
            step_name: String::new(),
            stream_filter: 0,
            skip: 0,
            take: 10,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.total, 1);
    assert!(resp.results[0].line.contains("needle"));
}

#[tokio::test]
async fn auth_interceptor_sends_bearer_header() {
    let (server, seen) = spawn_stub().await;
    let mut client = build_client(&server, "ory_at_test_token").await.unwrap();
    client
        .list_definitions(ListDefinitionsRequest {})
        .await
        .unwrap();
    let captured = seen.lock().await.clone();
    assert_eq!(captured, Some("Bearer ory_at_test_token".to_string()));
}

#[tokio::test]
async fn empty_token_sends_no_header() {
    let (server, seen) = spawn_stub().await;
    let mut client = build_client(&server, "").await.unwrap();
    client
        .list_definitions(ListDefinitionsRequest {})
        .await
        .unwrap();
    let captured = seen.lock().await.clone();
    assert!(captured.is_none());
}
