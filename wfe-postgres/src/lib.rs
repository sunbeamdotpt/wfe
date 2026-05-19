//! wfe-postgres — PostgreSQL persistence provider for the WFE workflow engine.
use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use wfe_core::models::{
    CommandName, Event, EventSubscription, ExecutionError, ExecutionPointer, PointerStatus,
    ScheduledCommand, WorkflowInstance, WorkflowStatus,
};
use wfe_core::traits::{
    EventRepository, PersistenceProvider, ScheduledCommandRepository, SubscriptionRepository,
    WorkflowRepository,
};
use wfe_core::{Result, WfeError};

/// PostgreSQL-backed persistence provider for the WFE workflow engine.
///
/// Stores workflows, execution pointers, events, subscriptions, errors, and
/// scheduled commands in PostgreSQL tables under the `wfc` schema. The schema
/// is created automatically on first use.
pub struct PostgresPersistenceProvider {
    pool: PgPool,
}

impl PostgresPersistenceProvider {
    pub async fn new(database_url: &str) -> std::result::Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    /// Truncate all tables (for test cleanup).
    pub async fn truncate_all(&self) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            "TRUNCATE wfc.execution_errors, wfc.execution_pointers, wfc.event_subscriptions, wfc.events, wfc.scheduled_commands, wfc.workflows CASCADE"
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn map_sqlx_err(e: sqlx::Error) -> WfeError {
        WfeError::Persistence(e.to_string())
    }

    fn status_to_str(status: &WorkflowStatus) -> &'static str {
        match status {
            WorkflowStatus::Runnable => "Runnable",
            WorkflowStatus::Suspended => "Suspended",
            WorkflowStatus::Complete => "Complete",
            WorkflowStatus::Terminated => "Terminated",
        }
    }

    fn str_to_status(s: &str) -> Result<WorkflowStatus> {
        match s {
            "Runnable" => Ok(WorkflowStatus::Runnable),
            "Suspended" => Ok(WorkflowStatus::Suspended),
            "Complete" => Ok(WorkflowStatus::Complete),
            "Terminated" => Ok(WorkflowStatus::Terminated),
            other => Err(WfeError::Persistence(format!(
                "Unknown workflow status: {other}"
            ))),
        }
    }

    fn pointer_status_to_str(status: &PointerStatus) -> &'static str {
        match status {
            PointerStatus::Pending => "Pending",
            PointerStatus::Running => "Running",
            PointerStatus::Complete => "Complete",
            PointerStatus::Skipped => "Skipped",
            PointerStatus::Sleeping => "Sleeping",
            PointerStatus::WaitingForEvent => "WaitingForEvent",
            PointerStatus::Failed => "Failed",
            PointerStatus::Compensated => "Compensated",
            PointerStatus::Cancelled => "Cancelled",
            PointerStatus::PendingPredecessor => "PendingPredecessor",
        }
    }

    fn str_to_pointer_status(s: &str) -> Result<PointerStatus> {
        match s {
            "Pending" => Ok(PointerStatus::Pending),
            "Running" => Ok(PointerStatus::Running),
            "Complete" => Ok(PointerStatus::Complete),
            "Skipped" => Ok(PointerStatus::Skipped),
            "Sleeping" => Ok(PointerStatus::Sleeping),
            "WaitingForEvent" => Ok(PointerStatus::WaitingForEvent),
            "Failed" => Ok(PointerStatus::Failed),
            "Compensated" => Ok(PointerStatus::Compensated),
            "Cancelled" => Ok(PointerStatus::Cancelled),
            "PendingPredecessor" => Ok(PointerStatus::PendingPredecessor),
            other => Err(WfeError::Persistence(format!(
                "Unknown pointer status: {other}"
            ))),
        }
    }

    fn command_name_to_str(name: &CommandName) -> &'static str {
        match name {
            CommandName::ProcessWorkflow => "ProcessWorkflow",
            CommandName::ProcessEvent => "ProcessEvent",
        }
    }

    fn str_to_command_name(s: &str) -> Result<CommandName> {
        match s {
            "ProcessWorkflow" => Ok(CommandName::ProcessWorkflow),
            "ProcessEvent" => Ok(CommandName::ProcessEvent),
            other => Err(WfeError::Persistence(format!(
                "Unknown command name: {other}"
            ))),
        }
    }

    async fn insert_pointers(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        workflow_id: &str,
        pointers: &[ExecutionPointer],
    ) -> Result<()> {
        for p in pointers {
            let children_json = serde_json::to_value(&p.children)
                .map_err(|e| WfeError::Persistence(format!("Failed to serialize children: {e}")))?;
            let scope_json = serde_json::to_value(&p.scope)
                .map_err(|e| WfeError::Persistence(format!("Failed to serialize scope: {e}")))?;
            let ext_json = serde_json::to_value(&p.extension_attributes).map_err(|e| {
                WfeError::Persistence(format!("Failed to serialize extension_attributes: {e}"))
            })?;

            sqlx::query(
                r#"INSERT INTO wfc.execution_pointers
                   (id, workflow_id, step_id, active, status, sleep_until,
                    persistence_data, start_time, end_time, event_name, event_key,
                    event_published, event_data, step_name, retry_count, children,
                    context_item, predecessor_id, outcome, scope, extension_attributes)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)"#,
            )
            .bind(&p.id)
            .bind(workflow_id)
            .bind(p.step_id as i32)
            .bind(p.active)
            .bind(Self::pointer_status_to_str(&p.status))
            .bind(p.sleep_until)
            .bind(&p.persistence_data)
            .bind(p.start_time)
            .bind(p.end_time)
            .bind(&p.event_name)
            .bind(&p.event_key)
            .bind(p.event_published)
            .bind(&p.event_data)
            .bind(&p.step_name)
            .bind(p.retry_count as i32)
            .bind(&children_json)
            .bind(&p.context_item)
            .bind(&p.predecessor_id)
            .bind(&p.outcome)
            .bind(&scope_json)
            .bind(&ext_json)
            .execute(&mut **tx)
            .await
            .map_err(Self::map_sqlx_err)?;
        }
        Ok(())
    }

    async fn load_pointers(&self, workflow_id: &str) -> Result<Vec<ExecutionPointer>> {
        let rows = sqlx::query("SELECT * FROM wfc.execution_pointers WHERE workflow_id = $1")
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        let mut pointers = Vec::with_capacity(rows.len());
        for row in &rows {
            pointers.push(self.row_to_pointer(row)?);
        }
        Ok(pointers)
    }

    fn row_to_pointer(&self, row: &sqlx::postgres::PgRow) -> Result<ExecutionPointer> {
        let children_json: serde_json::Value = row.get("children");
        let scope_json: serde_json::Value = row.get("scope");
        let ext_json: serde_json::Value = row.get("extension_attributes");

        let children: Vec<String> = serde_json::from_value(children_json)
            .map_err(|e| WfeError::Persistence(format!("Failed to deserialize children: {e}")))?;
        let scope: Vec<String> = serde_json::from_value(scope_json)
            .map_err(|e| WfeError::Persistence(format!("Failed to deserialize scope: {e}")))?;
        let extension_attributes: HashMap<String, serde_json::Value> =
            serde_json::from_value(ext_json).map_err(|e| {
                WfeError::Persistence(format!("Failed to deserialize extension_attributes: {e}"))
            })?;

        let status_str: String = row.get("status");

        Ok(ExecutionPointer {
            id: row.get("id"),
            step_id: row.get::<i32, _>("step_id") as usize,
            active: row.get("active"),
            status: Self::str_to_pointer_status(&status_str)?,
            sleep_until: row.get("sleep_until"),
            persistence_data: row.get("persistence_data"),
            start_time: row.get("start_time"),
            end_time: row.get("end_time"),
            event_name: row.get("event_name"),
            event_key: row.get("event_key"),
            event_published: row.get("event_published"),
            event_data: row.get("event_data"),
            step_name: row.get("step_name"),
            retry_count: row.get::<i32, _>("retry_count") as u32,
            children,
            context_item: row.get("context_item"),
            predecessor_id: row.get("predecessor_id"),
            outcome: row.get("outcome"),
            scope,
            extension_attributes,
        })
    }
}

#[async_trait]
impl WorkflowRepository for PostgresPersistenceProvider {
    async fn create_new_workflow(&self, instance: &WorkflowInstance) -> Result<String> {
        let id = if instance.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            instance.id.clone()
        };
        // Fall back to the UUID when the caller didn't assign a human name.
        // In production `WorkflowHost::start_workflow` always fills this in
        // via `next_definition_sequence`, but test fixtures and any external
        // caller that forgets shouldn't trip the UNIQUE constraint.
        let name = if instance.name.is_empty() {
            id.clone()
        } else {
            instance.name.clone()
        };

        let mut tx = self.pool.begin().await.map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"INSERT INTO wfc.workflows
               (id, name, root_workflow_id, definition_id, version, description, reference,
                status, data, next_execution, create_time, complete_time)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
        )
        .bind(&id)
        .bind(&name)
        .bind(&instance.root_workflow_id)
        .bind(&instance.workflow_definition_id)
        .bind(instance.version as i32)
        .bind(&instance.description)
        .bind(&instance.reference)
        .bind(Self::status_to_str(&instance.status))
        .bind(&instance.data)
        .bind(instance.next_execution)
        .bind(instance.create_time)
        .bind(instance.complete_time)
        .execute(&mut *tx)
        .await
        .map_err(Self::map_sqlx_err)?;

        // Insert execution pointers
        self.insert_pointers(&mut tx, &id, &instance.execution_pointers)
            .await?;

        tx.commit().await.map_err(Self::map_sqlx_err)?;
        Ok(id)
    }

    async fn persist_workflow(&self, instance: &WorkflowInstance) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"UPDATE wfc.workflows SET
               name=$2, root_workflow_id=$3, definition_id=$4, version=$5,
               description=$6, reference=$7, status=$8, data=$9, next_execution=$10,
               create_time=$11, complete_time=$12,
               last_heartbeat_at=now()
               WHERE id=$1"#,
        )
        .bind(&instance.id)
        .bind(&instance.name)
        .bind(&instance.root_workflow_id)
        .bind(&instance.workflow_definition_id)
        .bind(instance.version as i32)
        .bind(&instance.description)
        .bind(&instance.reference)
        .bind(Self::status_to_str(&instance.status))
        .bind(&instance.data)
        .bind(instance.next_execution)
        .bind(instance.create_time)
        .bind(instance.complete_time)
        .execute(&mut *tx)
        .await
        .map_err(Self::map_sqlx_err)?;

        // Delete old pointers and re-insert
        sqlx::query("DELETE FROM wfc.execution_pointers WHERE workflow_id = $1")
            .bind(&instance.id)
            .execute(&mut *tx)
            .await
            .map_err(Self::map_sqlx_err)?;

        self.insert_pointers(&mut tx, &instance.id, &instance.execution_pointers)
            .await?;

        tx.commit().await.map_err(Self::map_sqlx_err)?;
        Ok(())
    }

    async fn persist_workflow_with_subscriptions(
        &self,
        instance: &WorkflowInstance,
        subscriptions: &[EventSubscription],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"UPDATE wfc.workflows SET
               name=$2, root_workflow_id=$3, definition_id=$4, version=$5,
               description=$6, reference=$7, status=$8, data=$9, next_execution=$10,
               create_time=$11, complete_time=$12,
               last_heartbeat_at=now()
               WHERE id=$1"#,
        )
        .bind(&instance.id)
        .bind(&instance.name)
        .bind(&instance.root_workflow_id)
        .bind(&instance.workflow_definition_id)
        .bind(instance.version as i32)
        .bind(&instance.description)
        .bind(&instance.reference)
        .bind(Self::status_to_str(&instance.status))
        .bind(&instance.data)
        .bind(instance.next_execution)
        .bind(instance.create_time)
        .bind(instance.complete_time)
        .execute(&mut *tx)
        .await
        .map_err(Self::map_sqlx_err)?;

        // Delete old pointers and re-insert
        sqlx::query("DELETE FROM wfc.execution_pointers WHERE workflow_id = $1")
            .bind(&instance.id)
            .execute(&mut *tx)
            .await
            .map_err(Self::map_sqlx_err)?;

        self.insert_pointers(&mut tx, &instance.id, &instance.execution_pointers)
            .await?;

        // Insert subscriptions
        for sub in subscriptions {
            let sub_id = if sub.id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                sub.id.clone()
            };
            sqlx::query(
                r#"INSERT INTO wfc.event_subscriptions
                   (id, workflow_id, step_id, execution_pointer_id, event_name, event_key,
                    subscribe_as_of, subscription_data, external_token, external_worker_id,
                    external_token_expiry)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
            )
            .bind(&sub_id)
            .bind(&sub.workflow_id)
            .bind(sub.step_id as i32)
            .bind(&sub.execution_pointer_id)
            .bind(&sub.event_name)
            .bind(&sub.event_key)
            .bind(sub.subscribe_as_of)
            .bind(&sub.subscription_data)
            .bind(&sub.external_token)
            .bind(&sub.external_worker_id)
            .bind(sub.external_token_expiry)
            .execute(&mut *tx)
            .await
            .map_err(Self::map_sqlx_err)?;
        }

        tx.commit().await.map_err(Self::map_sqlx_err)?;
        Ok(())
    }

    async fn get_runnable_instances(&self, as_at: DateTime<Utc>) -> Result<Vec<String>> {
        let as_at_millis = as_at.timestamp_millis();
        let rows = sqlx::query(
            "SELECT id FROM wfc.workflows WHERE status = 'Runnable' AND next_execution <= $1",
        )
        .bind(as_at_millis)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    async fn get_workflow_instance(&self, id: &str) -> Result<WorkflowInstance> {
        let row = sqlx::query("SELECT * FROM wfc.workflows WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?
            .ok_or_else(|| WfeError::WorkflowNotFound(id.to_string()))?;

        let pointers = self.load_pointers(id).await?;
        let status_str: String = row.get("status");

        Ok(WorkflowInstance {
            id: row.get("id"),
            name: row.get("name"),
            root_workflow_id: row.get("root_workflow_id"),
            workflow_definition_id: row.get("definition_id"),
            version: row.get::<i32, _>("version") as u32,
            description: row.get("description"),
            reference: row.get("reference"),
            execution_pointers: pointers,
            next_execution: row.get("next_execution"),
            status: Self::str_to_status(&status_str)?,
            data: row.get("data"),
            create_time: row.get("create_time"),
            complete_time: row.get("complete_time"),
        })
    }

    async fn get_workflow_instance_by_name(&self, name: &str) -> Result<WorkflowInstance> {
        let row = sqlx::query("SELECT id FROM wfc.workflows WHERE name = $1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?
            .ok_or_else(|| WfeError::WorkflowNotFound(name.to_string()))?;
        let id: String = row.get("id");
        self.get_workflow_instance(&id).await
    }

    async fn next_definition_sequence(&self, definition_id: &str) -> Result<u64> {
        // UPSERT the counter atomically and return the new value. `RETURNING`
        // gives us the post-increment number in a single round trip.
        let row = sqlx::query(
            r#"INSERT INTO wfc.definition_sequences (definition_id, next_num)
               VALUES ($1, 1)
               ON CONFLICT (definition_id) DO UPDATE
                 SET next_num = wfc.definition_sequences.next_num + 1
               RETURNING next_num"#,
        )
        .bind(definition_id)
        .fetch_one(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;
        let next: i64 = row.get("next_num");
        Ok(next as u64)
    }

    async fn get_workflow_instances(&self, ids: &[String]) -> Result<Vec<WorkflowInstance>> {
        let mut result = Vec::new();
        for id in ids {
            match self.get_workflow_instance(id).await {
                Ok(w) => result.push(w),
                Err(WfeError::WorkflowNotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(result)
    }
}

#[async_trait]
impl SubscriptionRepository for PostgresPersistenceProvider {
    async fn create_event_subscription(&self, subscription: &EventSubscription) -> Result<String> {
        let id = if subscription.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            subscription.id.clone()
        };

        sqlx::query(
            r#"INSERT INTO wfc.event_subscriptions
               (id, workflow_id, step_id, execution_pointer_id, event_name, event_key,
                subscribe_as_of, subscription_data, external_token, external_worker_id,
                external_token_expiry)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
        )
        .bind(&id)
        .bind(&subscription.workflow_id)
        .bind(subscription.step_id as i32)
        .bind(&subscription.execution_pointer_id)
        .bind(&subscription.event_name)
        .bind(&subscription.event_key)
        .bind(subscription.subscribe_as_of)
        .bind(&subscription.subscription_data)
        .bind(&subscription.external_token)
        .bind(&subscription.external_worker_id)
        .bind(subscription.external_token_expiry)
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(id)
    }

    async fn get_subscriptions(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<EventSubscription>> {
        let rows = sqlx::query(
            r#"SELECT * FROM wfc.event_subscriptions
               WHERE event_name = $1 AND event_key = $2
                 AND subscribe_as_of <= $3
                 AND external_token IS NULL"#,
        )
        .bind(event_name)
        .bind(event_key)
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| self.row_to_subscription(r)).collect())
    }

    async fn terminate_subscription(&self, subscription_id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM wfc.event_subscriptions WHERE id = $1")
            .bind(subscription_id)
            .execute(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::SubscriptionNotFound(subscription_id.to_string()));
        }
        Ok(())
    }

    async fn get_subscription(&self, subscription_id: &str) -> Result<EventSubscription> {
        let row = sqlx::query("SELECT * FROM wfc.event_subscriptions WHERE id = $1")
            .bind(subscription_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?
            .ok_or_else(|| WfeError::SubscriptionNotFound(subscription_id.to_string()))?;

        Ok(self.row_to_subscription(&row))
    }

    async fn get_first_open_subscription(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Option<EventSubscription>> {
        let row = sqlx::query(
            r#"SELECT * FROM wfc.event_subscriptions
               WHERE event_name = $1 AND event_key = $2
                 AND subscribe_as_of <= $3
                 AND external_token IS NULL
               LIMIT 1"#,
        )
        .bind(event_name)
        .bind(event_key)
        .bind(as_of)
        .fetch_optional(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(row.as_ref().map(|r| self.row_to_subscription(r)))
    }

    async fn set_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
        worker_id: &str,
        expiry: DateTime<Utc>,
    ) -> Result<bool> {
        // Only set if external_token IS NULL (CAS-style)
        let result = sqlx::query(
            r#"UPDATE wfc.event_subscriptions
               SET external_token = $2, external_worker_id = $3, external_token_expiry = $4
               WHERE id = $1 AND external_token IS NULL"#,
        )
        .bind(subscription_id)
        .bind(token)
        .bind(worker_id)
        .bind(expiry)
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        if result.rows_affected() == 0 {
            // Check if subscription exists
            let exists = sqlx::query("SELECT 1 FROM wfc.event_subscriptions WHERE id = $1")
                .bind(subscription_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(Self::map_sqlx_err)?;
            if exists.is_none() {
                return Err(WfeError::SubscriptionNotFound(subscription_id.to_string()));
            }
            return Ok(false);
        }
        Ok(true)
    }

    async fn clear_subscription_token(&self, subscription_id: &str, token: &str) -> Result<()> {
        let result = sqlx::query(
            r#"UPDATE wfc.event_subscriptions
               SET external_token = NULL, external_worker_id = NULL, external_token_expiry = NULL
               WHERE id = $1 AND external_token = $2"#,
        )
        .bind(subscription_id)
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::SubscriptionNotFound(subscription_id.to_string()));
        }
        Ok(())
    }
}

impl PostgresPersistenceProvider {
    fn row_to_subscription(&self, row: &sqlx::postgres::PgRow) -> EventSubscription {
        EventSubscription {
            id: row.get("id"),
            workflow_id: row.get("workflow_id"),
            step_id: row.get::<i32, _>("step_id") as usize,
            execution_pointer_id: row.get("execution_pointer_id"),
            event_name: row.get("event_name"),
            event_key: row.get("event_key"),
            subscribe_as_of: row.get("subscribe_as_of"),
            subscription_data: row.get("subscription_data"),
            external_token: row.get("external_token"),
            external_worker_id: row.get("external_worker_id"),
            external_token_expiry: row.get("external_token_expiry"),
        }
    }
}

#[async_trait]
impl EventRepository for PostgresPersistenceProvider {
    async fn create_event(&self, event: &Event) -> Result<String> {
        let id = if event.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.id.clone()
        };

        sqlx::query(
            r#"INSERT INTO wfc.events
               (id, event_name, event_key, event_data, event_time, is_processed)
               VALUES ($1,$2,$3,$4,$5,$6)"#,
        )
        .bind(&id)
        .bind(&event.event_name)
        .bind(&event.event_key)
        .bind(&event.event_data)
        .bind(event.event_time)
        .bind(event.is_processed)
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(id)
    }

    async fn get_event(&self, id: &str) -> Result<Event> {
        let row = sqlx::query("SELECT * FROM wfc.events WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?
            .ok_or_else(|| WfeError::EventNotFound(id.to_string()))?;

        Ok(Event {
            id: row.get("id"),
            event_name: row.get("event_name"),
            event_key: row.get("event_key"),
            event_data: row.get("event_data"),
            event_time: row.get("event_time"),
            is_processed: row.get("is_processed"),
        })
    }

    async fn get_runnable_events(&self, as_at: DateTime<Utc>) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT id FROM wfc.events WHERE is_processed = FALSE AND event_time <= $1",
        )
        .bind(as_at)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    async fn get_events(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        let rows = sqlx::query(
            r#"SELECT id FROM wfc.events
               WHERE event_name = $1 AND event_key = $2 AND event_time <= $3"#,
        )
        .bind(event_name)
        .bind(event_key)
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    async fn mark_event_processed(&self, id: &str) -> Result<()> {
        let result = sqlx::query("UPDATE wfc.events SET is_processed = TRUE WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::EventNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn mark_event_unprocessed(&self, id: &str) -> Result<()> {
        let result = sqlx::query("UPDATE wfc.events SET is_processed = FALSE WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::EventNotFound(id.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl ScheduledCommandRepository for PostgresPersistenceProvider {
    fn supports_scheduled_commands(&self) -> bool {
        true
    }

    async fn schedule_command(&self, command: &ScheduledCommand) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO wfc.scheduled_commands (command_name, data, execute_time)
               VALUES ($1, $2, $3)
               ON CONFLICT (command_name, data) DO UPDATE SET execute_time = $3"#,
        )
        .bind(Self::command_name_to_str(&command.command_name))
        .bind(&command.data)
        .bind(command.execute_time)
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(())
    }

    async fn process_commands(
        &self,
        as_of: DateTime<Utc>,
        handler: &(
             dyn Fn(
            ScheduledCommand,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
                 + Send
                 + Sync
         ),
    ) -> Result<()> {
        let as_of_millis = as_of.timestamp_millis();

        // 1. SELECT due commands (do not delete yet)
        let rows = sqlx::query("SELECT * FROM wfc.scheduled_commands WHERE execute_time <= $1")
            .bind(as_of_millis)
            .fetch_all(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        let commands: Vec<ScheduledCommand> = rows
            .iter()
            .map(|r| {
                let name_str: String = r.get("command_name");
                Ok(ScheduledCommand {
                    command_name: Self::str_to_command_name(&name_str)?,
                    data: r.get("data"),
                    execute_time: r.get("execute_time"),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        // 2. Process each command via the handler
        for cmd in commands {
            handler(cmd).await?;
        }

        // 3. Only delete processed commands after successful processing
        sqlx::query("DELETE FROM wfc.scheduled_commands WHERE execute_time <= $1")
            .bind(as_of_millis)
            .execute(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        Ok(())
    }
}

#[async_trait]
impl PersistenceProvider for PostgresPersistenceProvider {
    async fn persist_errors(&self, errors: &[ExecutionError]) -> Result<()> {
        for error in errors {
            sqlx::query(
                r#"INSERT INTO wfc.execution_errors
                   (error_time, workflow_id, execution_pointer_id, message)
                   VALUES ($1,$2,$3,$4)"#,
            )
            .bind(error.error_time)
            .bind(&error.workflow_id)
            .bind(&error.execution_pointer_id)
            .bind(&error.message)
            .execute(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;
        }
        Ok(())
    }

    async fn ensure_store_exists(&self) -> Result<()> {
        sqlx::query("CREATE SCHEMA IF NOT EXISTS wfc")
            .execute(&self.pool)
            .await
            .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                root_workflow_id TEXT,
                definition_id TEXT NOT NULL,
                version INT NOT NULL,
                description TEXT,
                reference TEXT,
                status TEXT NOT NULL,
                data JSONB NOT NULL DEFAULT '{}',
                next_execution BIGINT,
                create_time TIMESTAMPTZ NOT NULL,
                complete_time TIMESTAMPTZ
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        // Upgrade older databases that lack the `name` column. Back-fill with
        // the UUID so the NOT NULL + UNIQUE invariant holds retroactively;
        // callers can re-run with a real name on the next persist.
        sqlx::query(
            r#"DO $$
            BEGIN
                IF NOT EXISTS (
                    SELECT 1 FROM information_schema.columns
                    WHERE table_schema = 'wfc' AND table_name = 'workflows'
                      AND column_name = 'name'
                ) THEN
                    ALTER TABLE wfc.workflows ADD COLUMN name TEXT;
                    UPDATE wfc.workflows SET name = id WHERE name IS NULL;
                    ALTER TABLE wfc.workflows ALTER COLUMN name SET NOT NULL;
                    CREATE UNIQUE INDEX IF NOT EXISTS idx_workflows_name
                        ON wfc.workflows (name);
                END IF;
                IF NOT EXISTS (
                    SELECT 1 FROM information_schema.columns
                    WHERE table_schema = 'wfc' AND table_name = 'workflows'
                      AND column_name = 'root_workflow_id'
                ) THEN
                    ALTER TABLE wfc.workflows ADD COLUMN root_workflow_id TEXT;
                END IF;
            END$$;"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.definition_sequences (
                definition_id TEXT PRIMARY KEY,
                next_num BIGINT NOT NULL
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.execution_pointers (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL REFERENCES wfc.workflows(id),
                step_id INT NOT NULL,
                active BOOLEAN NOT NULL DEFAULT TRUE,
                status TEXT NOT NULL,
                sleep_until TIMESTAMPTZ,
                persistence_data JSONB,
                start_time TIMESTAMPTZ,
                end_time TIMESTAMPTZ,
                event_name TEXT,
                event_key TEXT,
                event_published BOOLEAN DEFAULT FALSE,
                event_data JSONB,
                step_name TEXT,
                retry_count INT DEFAULT 0,
                children JSONB DEFAULT '[]',
                context_item JSONB,
                predecessor_id TEXT,
                outcome JSONB,
                scope JSONB DEFAULT '[]',
                extension_attributes JSONB DEFAULT '{}'
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.events (
                id TEXT PRIMARY KEY,
                event_name TEXT NOT NULL,
                event_key TEXT NOT NULL,
                event_data JSONB NOT NULL DEFAULT 'null',
                event_time TIMESTAMPTZ NOT NULL,
                is_processed BOOLEAN DEFAULT FALSE
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.event_subscriptions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                step_id INT NOT NULL,
                execution_pointer_id TEXT NOT NULL,
                event_name TEXT NOT NULL,
                event_key TEXT NOT NULL,
                subscribe_as_of TIMESTAMPTZ NOT NULL,
                subscription_data JSONB,
                external_token TEXT,
                external_worker_id TEXT,
                external_token_expiry TIMESTAMPTZ
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.execution_errors (
                id SERIAL PRIMARY KEY,
                error_time TIMESTAMPTZ NOT NULL,
                workflow_id TEXT NOT NULL,
                execution_pointer_id TEXT NOT NULL,
                message TEXT NOT NULL
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        sqlx::query(
            r#"CREATE TABLE IF NOT EXISTS wfc.scheduled_commands (
                id SERIAL PRIMARY KEY,
                command_name TEXT NOT NULL,
                data TEXT NOT NULL,
                execute_time BIGINT NOT NULL,
                UNIQUE(command_name, data)
            )"#,
        )
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        // Create indexes (IF NOT EXISTS for idempotency)
        let indexes = [
            "CREATE INDEX IF NOT EXISTS idx_workflows_next_execution ON wfc.workflows (next_execution)",
            "CREATE INDEX IF NOT EXISTS idx_workflows_status ON wfc.workflows (status)",
            "CREATE INDEX IF NOT EXISTS idx_events_name_key ON wfc.events (event_name, event_key)",
            "CREATE INDEX IF NOT EXISTS idx_events_is_processed ON wfc.events (is_processed)",
            "CREATE INDEX IF NOT EXISTS idx_events_event_time ON wfc.events (event_time)",
            "CREATE INDEX IF NOT EXISTS idx_subscriptions_name_key ON wfc.event_subscriptions (event_name, event_key)",
            "CREATE INDEX IF NOT EXISTS idx_subscriptions_workflow ON wfc.event_subscriptions (workflow_id)",
            "CREATE INDEX IF NOT EXISTS idx_scheduled_commands_execute_time ON wfc.scheduled_commands (execute_time)",
        ];

        for idx in &indexes {
            sqlx::query(idx)
                .execute(&self.pool)
                .await
                .map_err(Self::map_sqlx_err)?;
        }

        Ok(())
    }
}
