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
        /// Field name to filter on.
        field: String,
        /// Only include records before this time.
        before: Option<chrono::DateTime<chrono::Utc>>,
        /// Only include records after this time.
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
    /// Index or re-index a workflow instance.
    async fn index_workflow(&self, instance: &WorkflowInstance) -> crate::Result<()>;
    /// Search workflows matching the given terms and filters.
    async fn search(
        &self,
        terms: &str,
        skip: u64,
        take: u64,
        filters: &[SearchFilter],
    ) -> crate::Result<Page<WorkflowSearchResult>>;
    /// Initialize the search index.
    async fn start(&self) -> crate::Result<()>;
    /// Shut down the search index.
    async fn stop(&self) -> crate::Result<()>;
}
