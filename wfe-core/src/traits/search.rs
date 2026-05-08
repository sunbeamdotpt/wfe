use async_trait::async_trait;

use crate::models::WorkflowInstance;

/// Result from a search query.
#[derive(Debug, Clone)]
pub struct WorkflowSearchResult {
    /// Id.
    pub id: String,
    /// Workflow definition id.
    pub workflow_definition_id: String,
    /// Version.
    pub version: u32,
    /// Status.
    pub status: crate::models::WorkflowStatus,
    /// Reference.
    pub reference: Option<String>,
    /// Description.
    pub description: Option<String>,
}

/// Filter for search queries.
#[derive(Debug, Clone)]
pub enum SearchFilter {
    /// Status.
    Status(crate::models::WorkflowStatus),
    /// Daterange.
    DateRange {
        field: String,
        before: Option<chrono::DateTime<chrono::Utc>>,
        after: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// Reference.
    Reference(String),
}

/// Paginated search results.
#[derive(Debug, Clone)]
pub struct Page<T> {
    /// Data.
    pub data: Vec<T>,
    /// Total.
    pub total: u64,
}

/// Search index for querying workflows.
#[async_trait]
pub trait SearchIndex: Send + Sync {
    async fn index_workflow(&self, instance: &WorkflowInstance) -> crate::Result<()>;
    async fn search(
        &self,
        terms: &str,
        skip: u64,
        take: u64,
        filters: &[SearchFilter],
    ) -> crate::Result<Page<WorkflowSearchResult>>;
    async fn start(&self) -> crate::Result<()>;
    async fn stop(&self) -> crate::Result<()>;
}
