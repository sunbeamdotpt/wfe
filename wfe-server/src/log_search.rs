use chrono::{DateTime, Utc};
use opensearch::http::transport::Transport;
use opensearch::{IndexParts, OpenSearch, SearchParts};
use serde::{Deserialize, Serialize};
use serde_json::json;
use wfe_core::traits::{LogChunk, LogStreamType};

const LOG_INDEX: &str = "wfe-build-logs";

/// Document structure for a log line stored in OpenSearch.
#[derive(Debug, Serialize, Deserialize)]
struct LogDocument {
    workflow_id: String,
    definition_id: String,
    step_id: usize,
    step_name: String,
    stream: String,
    line: String,
    timestamp: String,
}

impl LogDocument {
    fn from_chunk(chunk: &LogChunk) -> Self {
        Self {
            workflow_id: chunk.workflow_id.clone(),
            definition_id: chunk.definition_id.clone(),
            step_id: chunk.step_id,
            step_name: chunk.step_name.clone(),
            stream: match chunk.stream {
                LogStreamType::Stdout => "stdout".to_string(),
                LogStreamType::Stderr => "stderr".to_string(),
            },
            line: String::from_utf8_lossy(&chunk.data).trim_end().to_string(),
            timestamp: chunk.timestamp.to_rfc3339(),
        }
    }
}

/// Result from a log search query.
#[derive(Debug, Clone)]
pub struct LogSearchHit {
    pub workflow_id: String,
    pub definition_id: String,
    pub step_name: String,
    pub line: String,
    pub stream: String,
    pub timestamp: DateTime<Utc>,
}

/// OpenSearch-backed log search index.
pub struct LogSearchIndex {
    client: OpenSearch,
}

impl LogSearchIndex {
    pub fn new(url: &str) -> wfe_core::Result<Self> {
        let transport = Transport::single_node(url)
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
        Ok(Self {
            client: OpenSearch::new(transport),
        })
    }

    /// Create the log index if it doesn't exist.
    pub async fn ensure_index(&self) -> wfe_core::Result<()> {
        let exists = self
            .client
            .indices()
            .exists(opensearch::indices::IndicesExistsParts::Index(&[LOG_INDEX]))
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if exists.status_code().is_success() {
            return Ok(());
        }

        let body = json!({
            "mappings": {
                "properties": {
                    "workflow_id": { "type": "keyword" },
                    "definition_id": { "type": "keyword" },
                    "step_id": { "type": "integer" },
                    "step_name": { "type": "keyword" },
                    "stream": { "type": "keyword" },
                    "line": { "type": "text", "analyzer": "standard" },
                    "timestamp": { "type": "date" }
                }
            }
        });

        let response = self
            .client
            .indices()
            .create(opensearch::indices::IndicesCreateParts::Index(LOG_INDEX))
            .body(body)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if !response.status_code().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(wfe_core::WfeError::Persistence(format!(
                "Failed to create log index: {text}"
            )));
        }

        tracing::info!(index = LOG_INDEX, "log search index created");
        Ok(())
    }

    /// Index a single log chunk.
    pub async fn index_chunk(&self, chunk: &LogChunk) -> wfe_core::Result<()> {
        let doc = LogDocument::from_chunk(chunk);
        let body = serde_json::to_value(&doc)?;

        let response = self
            .client
            .index(IndexParts::Index(LOG_INDEX))
            .body(body)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if !response.status_code().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(wfe_core::WfeError::Persistence(format!(
                "failed to index log chunk: {text}"
            )));
        }

        Ok(())
    }

    /// Search log lines.
    pub async fn search(
        &self,
        query: &str,
        workflow_id: Option<&str>,
        step_name: Option<&str>,
        stream_filter: Option<&str>,
        skip: u64,
        take: u64,
    ) -> wfe_core::Result<(Vec<LogSearchHit>, u64)> {
        let mut must_clauses = Vec::new();
        let mut filter_clauses = Vec::new();

        if !query.is_empty() {
            must_clauses.push(json!({
                "match": { "line": query }
            }));
        }

        if let Some(wf_id) = workflow_id {
            filter_clauses.push(json!({ "term": { "workflow_id": wf_id } }));
        }
        if let Some(sn) = step_name {
            filter_clauses.push(json!({ "term": { "step_name": sn } }));
        }
        if let Some(stream) = stream_filter {
            filter_clauses.push(json!({ "term": { "stream": stream } }));
        }

        let query_body = if must_clauses.is_empty() && filter_clauses.is_empty() {
            json!({ "match_all": {} })
        } else {
            let mut bool_q = serde_json::Map::new();
            if !must_clauses.is_empty() {
                bool_q.insert("must".to_string(), json!(must_clauses));
            }
            if !filter_clauses.is_empty() {
                bool_q.insert("filter".to_string(), json!(filter_clauses));
            }
            json!({ "bool": bool_q })
        };

        let body = json!({
            "query": query_body,
            "from": skip,
            "size": take,
            "sort": [{ "timestamp": "asc" }]
        });

        let response = self
            .client
            .search(SearchParts::Index(&[LOG_INDEX]))
            .body(body)
            .send()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        if !response.status_code().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(wfe_core::WfeError::Persistence(format!(
                "Log search failed: {text}"
            )));
        }

        let resp_body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        let total = resp_body["hits"]["total"]["value"].as_u64().unwrap_or(0);
        let hits = resp_body["hits"]["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let results = hits
            .iter()
            .filter_map(|hit| {
                let src = &hit["_source"];
                Some(LogSearchHit {
                    workflow_id: src["workflow_id"].as_str()?.to_string(),
                    definition_id: src["definition_id"].as_str()?.to_string(),
                    step_name: src["step_name"].as_str()?.to_string(),
                    line: src["line"].as_str()?.to_string(),
                    stream: src["stream"].as_str()?.to_string(),
                    timestamp: src["timestamp"]
                        .as_str()
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(Utc::now),
                })
            })
            .collect();

        Ok((results, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_document_from_chunk_stdout() {
        let chunk = LogChunk {
            workflow_id: "wf-1".to_string(),
            definition_id: "ci".to_string(),
            step_id: 0,
            step_name: "build".to_string(),
            stream: LogStreamType::Stdout,
            data: b"compiling wfe-core\n".to_vec(),
            timestamp: Utc::now(),
        };
        let doc = LogDocument::from_chunk(&chunk);
        assert_eq!(doc.workflow_id, "wf-1");
        assert_eq!(doc.stream, "stdout");
        assert_eq!(doc.line, "compiling wfe-core");
        assert_eq!(doc.step_name, "build");
    }

    #[test]
    fn log_document_from_chunk_stderr() {
        let chunk = LogChunk {
            workflow_id: "wf-2".to_string(),
            definition_id: "deploy".to_string(),
            step_id: 1,
            step_name: "test".to_string(),
            stream: LogStreamType::Stderr,
            data: b"warning: unused variable\n".to_vec(),
            timestamp: Utc::now(),
        };
        let doc = LogDocument::from_chunk(&chunk);
        assert_eq!(doc.stream, "stderr");
        assert_eq!(doc.line, "warning: unused variable");
    }

    #[test]
    fn log_document_trims_trailing_newline() {
        let chunk = LogChunk {
            workflow_id: "wf-1".to_string(),
            definition_id: "ci".to_string(),
            step_id: 0,
            step_name: "build".to_string(),
            stream: LogStreamType::Stdout,
            data: b"line with newline\n".to_vec(),
            timestamp: Utc::now(),
        };
        let doc = LogDocument::from_chunk(&chunk);
        assert_eq!(doc.line, "line with newline");
    }

    #[test]
    fn log_document_serializes_to_json() {
        let chunk = LogChunk {
            workflow_id: "wf-1".to_string(),
            definition_id: "ci".to_string(),
            step_id: 2,
            step_name: "clippy".to_string(),
            stream: LogStreamType::Stdout,
            data: b"all good\n".to_vec(),
            timestamp: Utc::now(),
        };
        let doc = LogDocument::from_chunk(&chunk);
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["step_name"], "clippy");
        assert_eq!(json["step_id"], 2);
        assert!(json["timestamp"].is_string());
    }

    // ── OpenSearch integration tests ────────────────────────────────

    fn opensearch_url() -> Option<String> {
        let url =
            std::env::var("WFE_SEARCH_URL").unwrap_or_else(|_| "http://localhost:9200".to_string());
        // Quick TCP probe to check if OpenSearch is reachable.
        let addr = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("https://"))
            .unwrap_or("localhost:9200");
        match std::net::TcpStream::connect_timeout(
            &addr.parse().ok()?,
            std::time::Duration::from_secs(1),
        ) {
            Ok(_) => Some(url),
            Err(_) => None,
        }
    }

    fn make_test_chunk(
        workflow_id: &str,
        step_name: &str,
        stream: LogStreamType,
        line: &str,
    ) -> LogChunk {
        LogChunk {
            workflow_id: workflow_id.to_string(),
            definition_id: "test-def".to_string(),
            step_id: 0,
            step_name: step_name.to_string(),
            stream,
            data: format!("{line}\n").into_bytes(),
            timestamp: Utc::now(),
        }
    }

    /// Delete the test index to start clean.
    async fn cleanup_index(url: &str) {
        let client = reqwest::Client::new();
        let _ = client.delete(format!("{url}/{LOG_INDEX}")).send().await;
    }

    #[tokio::test]
    async fn opensearch_ensure_index_creates_index() {
        let Some(url) = opensearch_url() else {
            eprintln!("SKIP: OpenSearch not available");
            return;
        };
        cleanup_index(&url).await;

        let index = LogSearchIndex::new(&url).unwrap();
        index.ensure_index().await.unwrap();

        // Calling again should be idempotent.
        index.ensure_index().await.unwrap();

        cleanup_index(&url).await;
    }

    #[tokio::test]
    async fn opensearch_index_and_search_chunk() {
        let Some(url) = opensearch_url() else {
            eprintln!("SKIP: OpenSearch not available");
            return;
        };
        cleanup_index(&url).await;

        let index = LogSearchIndex::new(&url).unwrap();
        index.ensure_index().await.unwrap();

        // Index some log chunks.
        let chunk = make_test_chunk(
            "wf-search-1",
            "build",
            LogStreamType::Stdout,
            "compiling wfe-core v1.5.0",
        );
        index.index_chunk(&chunk).await.unwrap();

        let chunk = make_test_chunk(
            "wf-search-1",
            "build",
            LogStreamType::Stderr,
            "warning: unused variable",
        );
        index.index_chunk(&chunk).await.unwrap();

        let chunk = make_test_chunk(
            "wf-search-1",
            "test",
            LogStreamType::Stdout,
            "test result: ok. 79 passed",
        );
        index.index_chunk(&chunk).await.unwrap();

        // OpenSearch needs a refresh to make docs searchable.
        let client = reqwest::Client::new();
        client
            .post(format!("{url}/{LOG_INDEX}/_refresh"))
            .send()
            .await
            .unwrap();

        // Search by text.
        let (results, total) = index
            .search("wfe-core", None, None, None, 0, 10)
            .await
            .unwrap();
        assert!(total >= 1, "expected at least 1 hit, got {total}");
        assert!(results.iter().any(|r| r.line.contains("wfe-core")));

        // Search by workflow_id filter.
        let (results, _) = index
            .search("", Some("wf-search-1"), None, None, 0, 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 3);

        // Search by step_name filter.
        let (results, _) = index
            .search("", Some("wf-search-1"), Some("test"), None, 0, 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].line.contains("79 passed"));

        // Search by stream filter.
        let (results, _) = index
            .search("", Some("wf-search-1"), None, Some("stderr"), 0, 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].line.contains("unused variable"));

        cleanup_index(&url).await;
    }

    #[tokio::test]
    async fn opensearch_search_empty_index() {
        let Some(url) = opensearch_url() else {
            eprintln!("SKIP: OpenSearch not available");
            return;
        };
        cleanup_index(&url).await;

        let index = LogSearchIndex::new(&url).unwrap();
        index.ensure_index().await.unwrap();

        let (results, total) = index
            .search("nonexistent", None, None, None, 0, 10)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(results.is_empty());

        cleanup_index(&url).await;
    }

    #[tokio::test]
    async fn opensearch_search_pagination() {
        let Some(url) = opensearch_url() else {
            eprintln!("SKIP: OpenSearch not available");
            return;
        };
        cleanup_index(&url).await;

        let index = LogSearchIndex::new(&url).unwrap();
        index.ensure_index().await.unwrap();

        // Index 5 chunks.
        for i in 0..5 {
            let chunk = make_test_chunk(
                "wf-page",
                "build",
                LogStreamType::Stdout,
                &format!("line {i}"),
            );
            index.index_chunk(&chunk).await.unwrap();
        }

        let client = reqwest::Client::new();
        client
            .post(format!("{url}/{LOG_INDEX}/_refresh"))
            .send()
            .await
            .unwrap();

        // Get first 2.
        let (results, total) = index
            .search("", Some("wf-page"), None, None, 0, 2)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(results.len(), 2);

        // Get next 2.
        let (results, _) = index
            .search("", Some("wf-page"), None, None, 2, 2)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Get last 1.
        let (results, _) = index
            .search("", Some("wf-page"), None, None, 4, 2)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);

        cleanup_index(&url).await;
    }

    #[test]
    fn log_search_index_new_constructs_ok() {
        // Construction should succeed even for unreachable URLs (fails on first use).
        let result = LogSearchIndex::new("http://localhost:19876");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn opensearch_index_chunk_result_fields() {
        let Some(url) = opensearch_url() else {
            eprintln!("SKIP: OpenSearch not available");
            return;
        };
        cleanup_index(&url).await;

        let index = LogSearchIndex::new(&url).unwrap();
        index.ensure_index().await.unwrap();

        let chunk = make_test_chunk(
            "wf-fields",
            "clippy",
            LogStreamType::Stderr,
            "error: type mismatch",
        );
        index.index_chunk(&chunk).await.unwrap();

        let client = reqwest::Client::new();
        client
            .post(format!("{url}/{LOG_INDEX}/_refresh"))
            .send()
            .await
            .unwrap();

        let (results, _) = index
            .search("type mismatch", None, None, None, 0, 10)
            .await
            .unwrap();
        assert!(!results.is_empty());
        let hit = &results[0];
        assert_eq!(hit.workflow_id, "wf-fields");
        assert_eq!(hit.definition_id, "test-def");
        assert_eq!(hit.step_name, "clippy");
        assert_eq!(hit.stream, "stderr");
        assert!(hit.line.contains("type mismatch"));

        cleanup_index(&url).await;
    }
}
