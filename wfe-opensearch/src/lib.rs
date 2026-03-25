use async_trait::async_trait;
use opensearch::http::transport::Transport;
use opensearch::{IndexParts, OpenSearch, SearchParts};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::debug;
use wfe_core::models::{WorkflowInstance, WorkflowStatus};
use wfe_core::traits::search::{Page, SearchFilter, SearchIndex, WorkflowSearchResult};

/// Document structure stored in OpenSearch.
#[derive(Debug, Serialize, Deserialize)]
struct WorkflowDocument {
    id: String,
    workflow_definition_id: String,
    version: u32,
    status: String,
    reference: Option<String>,
    description: Option<String>,
    data: serde_json::Value,
    create_time: String,
    complete_time: Option<String>,
}

impl From<&WorkflowInstance> for WorkflowDocument {
    fn from(instance: &WorkflowInstance) -> Self {
        Self {
            id: instance.id.clone(),
            workflow_definition_id: instance.workflow_definition_id.clone(),
            version: instance.version,
            status: serde_json::to_value(instance.status)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default(),
            reference: instance.reference.clone(),
            description: instance.description.clone(),
            data: instance.data.clone(),
            create_time: instance.create_time.to_rfc3339(),
            complete_time: instance.complete_time.map(|t| t.to_rfc3339()),
        }
    }
}

/// OpenSearch-backed search index for workflow instances.
pub struct OpenSearchIndex {
    client: OpenSearch,
    index_name: String,
}

impl OpenSearchIndex {
    /// Create a new OpenSearch index provider.
    ///
    /// # Arguments
    /// * `url` - OpenSearch server URL (e.g. `http://localhost:9200`)
    /// * `index_name` - Name of the index to use
    pub fn new(url: &str, index_name: &str) -> wfe_core::Result<Self> {
        let transport = Transport::single_node(url)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        let client = OpenSearch::new(transport);
        Ok(Self {
            client,
            index_name: index_name.to_string(),
        })
    }

    /// Get a reference to the underlying OpenSearch client (useful for tests).
    pub fn client(&self) -> &OpenSearch {
        &self.client
    }

    /// Get the index name.
    pub fn index_name(&self) -> &str {
        &self.index_name
    }
}

#[async_trait]
impl SearchIndex for OpenSearchIndex {
    async fn start(&self) -> wfe_core::Result<()> {
        let exists = self
            .client
            .indices()
            .exists(opensearch::indices::IndicesExistsParts::Index(&[
                &self.index_name,
            ]))
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if exists.status_code().is_success() {
            debug!(index = %self.index_name, "Index already exists");
            return Ok(());
        }

        let body = json!({
            "mappings": {
                "properties": {
                    "id": { "type": "keyword" },
                    "workflow_definition_id": { "type": "keyword" },
                    "version": { "type": "integer" },
                    "status": { "type": "keyword" },
                    "reference": { "type": "keyword" },
                    "description": { "type": "text" },
                    "data": { "type": "object", "enabled": false },
                    "create_time": { "type": "date" },
                    "complete_time": { "type": "date" }
                }
            }
        });

        let response = self
            .client
            .indices()
            .create(opensearch::indices::IndicesCreateParts::Index(
                &self.index_name,
            ))
            .body(body)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if !response.status_code().is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(wfe_core::WfeError::Persistence(format!(
                "Failed to create index: {text}"
            )));
        }

        debug!(index = %self.index_name, "Index created");
        Ok(())
    }

    async fn stop(&self) -> wfe_core::Result<()> {
        Ok(())
    }

    async fn index_workflow(&self, instance: &WorkflowInstance) -> wfe_core::Result<()> {
        let doc = WorkflowDocument::from(instance);

        let response = self
            .client
            .index(IndexParts::IndexId(&self.index_name, &doc.id))
            .body(serde_json::to_value(&doc)?)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if !response.status_code().is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(wfe_core::WfeError::Persistence(format!(
                "Failed to index workflow: {text}"
            )));
        }

        debug!(id = %instance.id, index = %self.index_name, "Workflow indexed");
        Ok(())
    }

    async fn search(
        &self,
        terms: &str,
        skip: u64,
        take: u64,
        filters: &[SearchFilter],
    ) -> wfe_core::Result<Page<WorkflowSearchResult>> {
        let mut must_clauses: Vec<serde_json::Value> = Vec::new();
        let mut filter_clauses: Vec<serde_json::Value> = Vec::new();

        if !terms.is_empty() {
            must_clauses.push(json!({
                "multi_match": {
                    "query": terms,
                    "fields": ["description", "reference", "workflow_definition_id"]
                }
            }));
        }

        for filter in filters {
            match filter {
                SearchFilter::Status(status) => {
                    let status_str = serde_json::to_value(status)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default();
                    filter_clauses.push(json!({
                        "term": { "status": status_str }
                    }));
                }
                SearchFilter::DateRange {
                    field,
                    before,
                    after,
                } => {
                    let mut range = serde_json::Map::new();
                    if let Some(before) = before {
                        range.insert("lt".to_string(), json!(before.to_rfc3339()));
                    }
                    if let Some(after) = after {
                        range.insert("gte".to_string(), json!(after.to_rfc3339()));
                    }
                    if !range.is_empty() {
                        filter_clauses.push(json!({
                            "range": { field.clone(): range }
                        }));
                    }
                }
                SearchFilter::Reference(reference) => {
                    filter_clauses.push(json!({
                        "term": { "reference": reference }
                    }));
                }
            }
        }

        let query = if must_clauses.is_empty() && filter_clauses.is_empty() {
            json!({ "match_all": {} })
        } else {
            let mut bool_query = serde_json::Map::new();
            if !must_clauses.is_empty() {
                bool_query.insert("must".to_string(), json!(must_clauses));
            }
            if !filter_clauses.is_empty() {
                bool_query.insert("filter".to_string(), json!(filter_clauses));
            }
            json!({ "bool": bool_query })
        };

        let body = json!({
            "query": query,
            "from": skip,
            "size": take,
        });

        let response = self
            .client
            .search(SearchParts::Index(&[&self.index_name]))
            .body(body)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if !response.status_code().is_success() {
            let text = response
                .text()
                .await
                .unwrap_or_else(|_| "unknown error".to_string());
            return Err(wfe_core::WfeError::Persistence(format!(
                "Search failed: {text}"
            )));
        }

        let response_body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let total = response_body["hits"]["total"]["value"]
            .as_u64()
            .unwrap_or(0);

        let hits = response_body["hits"]["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut results = Vec::with_capacity(hits.len());
        for hit in &hits {
            let source = &hit["_source"];
            let status_str = source["status"].as_str().unwrap_or("Runnable");
            let status: WorkflowStatus =
                serde_json::from_value(json!(status_str)).unwrap_or_default();

            results.push(WorkflowSearchResult {
                id: source["id"].as_str().unwrap_or_default().to_string(),
                workflow_definition_id: source["workflow_definition_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                version: source["version"].as_u64().unwrap_or(0) as u32,
                status,
                reference: source["reference"].as_str().map(String::from),
                description: source["description"].as_str().map(String::from),
            });
        }

        Ok(Page {
            data: results,
            total,
        })
    }
}
