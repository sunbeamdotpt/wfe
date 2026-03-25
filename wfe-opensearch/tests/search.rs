use chrono::Utc;
use opensearch::http::transport::Transport;
use opensearch::OpenSearch;
use pretty_assertions::assert_eq;
use serde_json::json;
use uuid::Uuid;
use wfe_core::models::{WorkflowInstance, WorkflowStatus};
use wfe_core::traits::search::{SearchFilter, SearchIndex};
use wfe_opensearch::OpenSearchIndex;

const OPENSEARCH_URL: &str = "http://localhost:9200";

/// Check if OpenSearch is reachable, skip test if not.
async fn opensearch_available() -> bool {
    let transport = Transport::single_node(OPENSEARCH_URL);
    if transport.is_err() {
        return false;
    }
    let client = OpenSearch::new(transport.unwrap());
    client
        .ping()
        .send()
        .await
        .map(|r| r.status_code().is_success())
        .unwrap_or(false)
}

/// Helper to create a unique test index and return the provider + cleanup handle.
async fn setup() -> Option<(OpenSearchIndex, String)> {
    if !opensearch_available().await {
        eprintln!("OpenSearch not available, skipping test");
        return None;
    }
    let index_name = format!("wfe_test_{}", Uuid::new_v4());
    let provider = OpenSearchIndex::new(OPENSEARCH_URL, &index_name).unwrap();
    provider.start().await.unwrap();
    Some((provider, index_name))
}

/// Refresh the index so documents become searchable immediately.
async fn refresh_index(provider: &OpenSearchIndex) {
    let url = format!("/{}/_refresh", provider.index_name());
    provider
        .client()
        .send(
            opensearch::http::Method::Post,
            &url,
            opensearch::http::headers::HeaderMap::new(),
            Option::<&serde_json::Value>::None,
            Some(b"".as_ref()),
            None,
        )
        .await
        .unwrap();
}

/// Delete the test index.
async fn cleanup(provider: &OpenSearchIndex) {
    let _ = provider
        .client()
        .indices()
        .delete(opensearch::indices::IndicesDeleteParts::Index(&[
            provider.index_name(),
        ]))
        .send()
        .await;
}

fn make_instance(description: Option<&str>, reference: Option<&str>) -> WorkflowInstance {
    let mut instance = WorkflowInstance::new("test-workflow", 1, json!({"key": "value"}));
    instance.description = description.map(String::from);
    instance.reference = reference.map(String::from);
    instance
}

#[tokio::test]
async fn index_and_search_by_terms() {
    let Some((provider, _index)) = setup().await else {
        return;
    };

    let instance = make_instance(Some("Process the quarterly financial report"), None);
    provider.index_workflow(&instance).await.unwrap();
    refresh_index(&provider).await;

    let page = provider
        .search("quarterly financial", 0, 10, &[])
        .await
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].id, instance.id);
    assert_eq!(
        page.data[0].description.as_deref(),
        Some("Process the quarterly financial report")
    );

    cleanup(&provider).await;
}

#[tokio::test]
async fn search_with_status_filter() {
    let Some((provider, _index)) = setup().await else {
        return;
    };

    let mut runnable = make_instance(Some("Runnable workflow"), None);
    runnable.status = WorkflowStatus::Runnable;

    let mut complete = make_instance(Some("Complete workflow"), None);
    complete.status = WorkflowStatus::Complete;
    complete.complete_time = Some(Utc::now());

    provider.index_workflow(&runnable).await.unwrap();
    provider.index_workflow(&complete).await.unwrap();
    refresh_index(&provider).await;

    let page = provider
        .search("", 0, 10, &[SearchFilter::Status(WorkflowStatus::Complete)])
        .await
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.data[0].id, complete.id);
    assert_eq!(page.data[0].status, WorkflowStatus::Complete);

    cleanup(&provider).await;
}

#[tokio::test]
async fn search_with_no_results() {
    let Some((provider, _index)) = setup().await else {
        return;
    };

    let instance = make_instance(Some("A regular workflow"), None);
    provider.index_workflow(&instance).await.unwrap();
    refresh_index(&provider).await;

    let page = provider
        .search("nonexistent-xyzzy-42", 0, 10, &[])
        .await
        .unwrap();

    assert_eq!(page.total, 0);
    assert!(page.data.is_empty());

    cleanup(&provider).await;
}

#[tokio::test]
async fn index_multiple_and_paginate() {
    let Some((provider, _index)) = setup().await else {
        return;
    };

    let mut instances = Vec::new();
    for i in 0..5 {
        let instance = make_instance(Some(&format!("Paginated workflow number {i}")), None);
        provider.index_workflow(&instance).await.unwrap();
        instances.push(instance);
    }
    refresh_index(&provider).await;

    // Search all, but skip 2 and take 2
    let page = provider.search("Paginated workflow", 2, 2, &[]).await.unwrap();

    assert_eq!(page.total, 5);
    assert_eq!(page.data.len(), 2);

    cleanup(&provider).await;
}

#[tokio::test]
async fn search_by_reference() {
    let Some((provider, _index)) = setup().await else {
        return;
    };

    let inst1 = make_instance(Some("First workflow"), Some("REF-001"));
    let inst2 = make_instance(Some("Second workflow"), Some("REF-002"));

    provider.index_workflow(&inst1).await.unwrap();
    provider.index_workflow(&inst2).await.unwrap();
    refresh_index(&provider).await;

    let page = provider
        .search("", 0, 10, &[SearchFilter::Reference("REF-001".to_string())])
        .await
        .unwrap();

    assert_eq!(page.total, 1);
    assert_eq!(page.data[0].id, inst1.id);
    assert_eq!(page.data[0].reference.as_deref(), Some("REF-001"));

    cleanup(&provider).await;
}
