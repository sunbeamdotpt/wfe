use chrono::{DateTime, Utc};

use wfe_core::models::WorkflowStatus;
use wfe_core::traits::PersistenceProvider;
use wfe_core::Result;

/// Purge workflows matching a given status that were created before `older_than`.
///
/// TODO: This requires a query-by-status-and-date method on the persistence trait.
/// For now, this is a stub that documents the intended contract. When the persistence
/// layer gains `get_workflows_by_status` and `delete_workflow` methods, this function
/// should be implemented fully.
pub async fn purge_workflows(
    _persistence: &dyn PersistenceProvider,
    _status: WorkflowStatus,
    _older_than: DateTime<Utc>,
) -> Result<()> {
    // TODO: Implement once PersistenceProvider exposes:
    //   async fn get_workflows_by_status(&self, status: WorkflowStatus, before: DateTime<Utc>) -> Result<Vec<String>>;
    //   async fn delete_workflow(&self, id: &str) -> Result<()>;
    //
    // Intended implementation:
    //   let ids = persistence.get_workflows_by_status(status, older_than).await?;
    //   for id in ids {
    //       persistence.delete_workflow(&id).await?;
    //   }
    Ok(())
}
