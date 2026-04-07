use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use wfe_core::models::{
    CommandName, Event, EventSubscription, ExecutionError, ExecutionPointer, ScheduledCommand,
    WorkflowInstance, WorkflowStatus,
};
use wfe_core::traits::{
    EventRepository, PersistenceProvider, ScheduledCommandRepository, SubscriptionRepository,
    WorkflowRepository,
};
use wfe_core::{Result, WfeError};

/// SQLite-backed persistence provider for the WFE workflow engine.
pub struct SqlitePersistenceProvider {
    pool: SqlitePool,
}

impl SqlitePersistenceProvider {
    /// Create a new provider connected to the given SQLite database URL.
    ///
    /// For in-memory databases, pass `":memory:"`.
    /// The schema tables are created automatically.
    pub async fn new(database_url: &str) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let options: SqliteConnectOptions = database_url
            .parse::<SqliteConnectOptions>()?
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let max_connections = if database_url.contains(":memory:") {
            1
        } else {
            4
        };

        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await?;

        // Enable WAL mode and foreign keys
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await?;

        let provider = Self { pool };
        provider.ensure_store_exists().await?;
        Ok(provider)
    }

    /// Run the DDL statements to create tables and indexes.
    async fn create_tables(&self) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                definition_id TEXT NOT NULL,
                version INTEGER NOT NULL,
                description TEXT,
                reference TEXT,
                status TEXT NOT NULL,
                data TEXT NOT NULL,
                next_execution INTEGER,
                create_time TEXT NOT NULL,
                complete_time TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        // Per-definition monotonic counter used to generate human-friendly
        // instance names of the form `{definition_id}-{N}`.
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS definition_sequences (
                definition_id TEXT PRIMARY KEY,
                next_num INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS execution_pointers (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                step_id INTEGER NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                status TEXT NOT NULL,
                sleep_until TEXT,
                persistence_data TEXT,
                start_time TEXT,
                end_time TEXT,
                event_name TEXT,
                event_key TEXT,
                event_published INTEGER NOT NULL DEFAULT 0,
                event_data TEXT,
                step_name TEXT,
                retry_count INTEGER NOT NULL DEFAULT 0,
                children TEXT NOT NULL DEFAULT '[]',
                context_item TEXT,
                predecessor_id TEXT,
                outcome TEXT,
                scope TEXT NOT NULL DEFAULT '[]',
                extension_attributes TEXT NOT NULL DEFAULT '{}',
                FOREIGN KEY (workflow_id) REFERENCES workflows(id) ON DELETE CASCADE
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                event_name TEXT NOT NULL,
                event_key TEXT NOT NULL,
                event_data TEXT NOT NULL,
                event_time TEXT NOT NULL,
                is_processed INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS event_subscriptions (
                id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                step_id INTEGER NOT NULL,
                execution_pointer_id TEXT NOT NULL,
                event_name TEXT NOT NULL,
                event_key TEXT NOT NULL,
                subscribe_as_of TEXT NOT NULL,
                subscription_data TEXT,
                external_token TEXT,
                external_worker_id TEXT,
                external_token_expiry TEXT,
                terminated INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS execution_errors (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                error_time TEXT NOT NULL,
                workflow_id TEXT NOT NULL,
                execution_pointer_id TEXT NOT NULL,
                message TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS scheduled_commands (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command_name TEXT NOT NULL,
                data TEXT NOT NULL,
                execute_time INTEGER NOT NULL,
                UNIQUE(command_name, data)
            )",
        )
        .execute(&self.pool)
        .await?;

        // Indexes
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_workflows_next_execution ON workflows(next_execution)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_workflows_status ON workflows(status)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_execution_pointers_workflow_id ON execution_pointers(workflow_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_events_name_key ON events(event_name, event_key)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_is_processed ON events(is_processed)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_events_event_time ON events(event_time)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_event_subscriptions_name_key ON event_subscriptions(event_name, event_key)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_event_subscriptions_workflow_id ON event_subscriptions(workflow_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_scheduled_commands_execute_time ON scheduled_commands(execute_time)")
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn to_persistence_err(e: sqlx::Error) -> WfeError {
    WfeError::Persistence(e.to_string())
}

fn dt_to_string(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

fn opt_dt_to_string(dt: &Option<DateTime<Utc>>) -> Option<String> {
    dt.as_ref().map(dt_to_string)
}

fn string_to_dt(s: &str) -> std::result::Result<DateTime<Utc>, WfeError> {
    s.parse::<DateTime<Utc>>()
        .map_err(|e| WfeError::Persistence(format!("Failed to parse datetime '{s}': {e}")))
}

fn string_to_opt_dt(s: &Option<String>) -> std::result::Result<Option<DateTime<Utc>>, WfeError> {
    match s {
        Some(s) => Ok(Some(string_to_dt(s)?)),
        None => Ok(None),
    }
}

fn row_to_workflow(
    row: &sqlx::sqlite::SqliteRow,
    pointers: Vec<ExecutionPointer>,
) -> std::result::Result<WorkflowInstance, WfeError> {
    let status_str: String = row.try_get("status").map_err(to_persistence_err)?;
    let status: WorkflowStatus = serde_json::from_str(&format!("\"{status_str}\""))
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize WorkflowStatus: {e}")))?;

    let data_str: String = row.try_get("data").map_err(to_persistence_err)?;
    let data: serde_json::Value = serde_json::from_str(&data_str)
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize data: {e}")))?;

    let create_time_str: String = row.try_get("create_time").map_err(to_persistence_err)?;
    let complete_time_str: Option<String> =
        row.try_get("complete_time").map_err(to_persistence_err)?;

    Ok(WorkflowInstance {
        id: row.try_get("id").map_err(to_persistence_err)?,
        name: row.try_get("name").map_err(to_persistence_err)?,
        workflow_definition_id: row.try_get("definition_id").map_err(to_persistence_err)?,
        version: row
            .try_get::<i64, _>("version")
            .map_err(to_persistence_err)? as u32,
        description: row.try_get("description").map_err(to_persistence_err)?,
        reference: row.try_get("reference").map_err(to_persistence_err)?,
        execution_pointers: pointers,
        next_execution: row.try_get("next_execution").map_err(to_persistence_err)?,
        status,
        data,
        create_time: string_to_dt(&create_time_str)?,
        complete_time: string_to_opt_dt(&complete_time_str)?,
    })
}

fn row_to_pointer(
    row: &sqlx::sqlite::SqliteRow,
) -> std::result::Result<ExecutionPointer, WfeError> {
    let status_str: String = row.try_get("status").map_err(to_persistence_err)?;
    let status: wfe_core::models::PointerStatus =
        serde_json::from_str(&format!("\"{status_str}\"")).map_err(|e| {
            WfeError::Persistence(format!("Failed to deserialize PointerStatus: {e}"))
        })?;

    let persistence_data_str: Option<String> = row
        .try_get("persistence_data")
        .map_err(to_persistence_err)?;
    let persistence_data: Option<serde_json::Value> = persistence_data_str
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| {
            WfeError::Persistence(format!("Failed to deserialize persistence_data: {e}"))
        })?;

    let event_data_str: Option<String> = row.try_get("event_data").map_err(to_persistence_err)?;
    let event_data: Option<serde_json::Value> = event_data_str
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize event_data: {e}")))?;

    let context_item_str: Option<String> =
        row.try_get("context_item").map_err(to_persistence_err)?;
    let context_item: Option<serde_json::Value> = context_item_str
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize context_item: {e}")))?;

    let outcome_str: Option<String> = row.try_get("outcome").map_err(to_persistence_err)?;
    let outcome: Option<serde_json::Value> = outcome_str
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize outcome: {e}")))?;

    let children_str: String = row.try_get("children").map_err(to_persistence_err)?;
    let children: Vec<String> = serde_json::from_str(&children_str)
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize children: {e}")))?;

    let scope_str: String = row.try_get("scope").map_err(to_persistence_err)?;
    let scope: Vec<String> = serde_json::from_str(&scope_str)
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize scope: {e}")))?;

    let ext_str: String = row
        .try_get("extension_attributes")
        .map_err(to_persistence_err)?;
    let extension_attributes: HashMap<String, serde_json::Value> = serde_json::from_str(&ext_str)
        .map_err(|e| {
        WfeError::Persistence(format!("Failed to deserialize extension_attributes: {e}"))
    })?;

    let sleep_until_str: Option<String> = row.try_get("sleep_until").map_err(to_persistence_err)?;
    let start_time_str: Option<String> = row.try_get("start_time").map_err(to_persistence_err)?;
    let end_time_str: Option<String> = row.try_get("end_time").map_err(to_persistence_err)?;

    Ok(ExecutionPointer {
        id: row.try_get("id").map_err(to_persistence_err)?,
        step_id: row
            .try_get::<i64, _>("step_id")
            .map_err(to_persistence_err)? as usize,
        active: row
            .try_get::<bool, _>("active")
            .map_err(to_persistence_err)?,
        status,
        sleep_until: string_to_opt_dt(&sleep_until_str)?,
        persistence_data,
        start_time: string_to_opt_dt(&start_time_str)?,
        end_time: string_to_opt_dt(&end_time_str)?,
        event_name: row.try_get("event_name").map_err(to_persistence_err)?,
        event_key: row.try_get("event_key").map_err(to_persistence_err)?,
        event_published: row
            .try_get::<bool, _>("event_published")
            .map_err(to_persistence_err)?,
        event_data,
        step_name: row.try_get("step_name").map_err(to_persistence_err)?,
        retry_count: row
            .try_get::<i64, _>("retry_count")
            .map_err(to_persistence_err)? as u32,
        children,
        context_item,
        predecessor_id: row.try_get("predecessor_id").map_err(to_persistence_err)?,
        outcome,
        scope,
        extension_attributes,
    })
}

fn row_to_event(row: &sqlx::sqlite::SqliteRow) -> std::result::Result<Event, WfeError> {
    let event_data_str: String = row.try_get("event_data").map_err(to_persistence_err)?;
    let event_data: serde_json::Value = serde_json::from_str(&event_data_str)
        .map_err(|e| WfeError::Persistence(format!("Failed to deserialize event_data: {e}")))?;

    let event_time_str: String = row.try_get("event_time").map_err(to_persistence_err)?;

    Ok(Event {
        id: row.try_get("id").map_err(to_persistence_err)?,
        event_name: row.try_get("event_name").map_err(to_persistence_err)?,
        event_key: row.try_get("event_key").map_err(to_persistence_err)?,
        event_data,
        event_time: string_to_dt(&event_time_str)?,
        is_processed: row
            .try_get::<bool, _>("is_processed")
            .map_err(to_persistence_err)?,
    })
}

fn row_to_subscription(
    row: &sqlx::sqlite::SqliteRow,
) -> std::result::Result<EventSubscription, WfeError> {
    let subscribe_as_of_str: String = row.try_get("subscribe_as_of").map_err(to_persistence_err)?;

    let subscription_data_str: Option<String> = row
        .try_get("subscription_data")
        .map_err(to_persistence_err)?;
    let subscription_data: Option<serde_json::Value> = subscription_data_str
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| {
            WfeError::Persistence(format!("Failed to deserialize subscription_data: {e}"))
        })?;

    let external_token_expiry_str: Option<String> = row
        .try_get("external_token_expiry")
        .map_err(to_persistence_err)?;

    Ok(EventSubscription {
        id: row.try_get("id").map_err(to_persistence_err)?,
        workflow_id: row.try_get("workflow_id").map_err(to_persistence_err)?,
        step_id: row
            .try_get::<i64, _>("step_id")
            .map_err(to_persistence_err)? as usize,
        execution_pointer_id: row
            .try_get("execution_pointer_id")
            .map_err(to_persistence_err)?,
        event_name: row.try_get("event_name").map_err(to_persistence_err)?,
        event_key: row.try_get("event_key").map_err(to_persistence_err)?,
        subscribe_as_of: string_to_dt(&subscribe_as_of_str)?,
        subscription_data,
        external_token: row.try_get("external_token").map_err(to_persistence_err)?,
        external_worker_id: row
            .try_get("external_worker_id")
            .map_err(to_persistence_err)?,
        external_token_expiry: string_to_opt_dt(&external_token_expiry_str)?,
    })
}

// ─── Trait implementations ─────────────────────────────────────────────────

#[async_trait]
impl WorkflowRepository for SqlitePersistenceProvider {
    async fn create_new_workflow(&self, instance: &WorkflowInstance) -> Result<String> {
        let id = if instance.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            instance.id.clone()
        };

        let status_str = serde_json::to_value(instance.status)
            .map_err(|e| WfeError::Persistence(e.to_string()))?
            .as_str()
            .unwrap_or("Runnable")
            .to_string();
        let data_str = serde_json::to_string(&instance.data)
            .map_err(|e| WfeError::Persistence(e.to_string()))?;
        let create_time_str = dt_to_string(&instance.create_time);
        let complete_time_str = opt_dt_to_string(&instance.complete_time);

        let mut tx = self.pool.begin().await.map_err(to_persistence_err)?;

        sqlx::query(
            "INSERT INTO workflows (id, name, definition_id, version, description, reference, status, data, next_execution, create_time, complete_time)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(&id)
        .bind(&instance.name)
        .bind(&instance.workflow_definition_id)
        .bind(instance.version as i64)
        .bind(&instance.description)
        .bind(&instance.reference)
        .bind(&status_str)
        .bind(&data_str)
        .bind(instance.next_execution)
        .bind(&create_time_str)
        .bind(&complete_time_str)
        .execute(&mut *tx)
        .await
        .map_err(to_persistence_err)?;

        for ptr in &instance.execution_pointers {
            insert_pointer(&mut tx, &id, ptr).await?;
        }

        tx.commit().await.map_err(to_persistence_err)?;
        Ok(id)
    }

    async fn persist_workflow(&self, instance: &WorkflowInstance) -> Result<()> {
        let status_str = serde_json::to_value(instance.status)
            .map_err(|e| WfeError::Persistence(e.to_string()))?
            .as_str()
            .unwrap_or("Runnable")
            .to_string();
        let data_str = serde_json::to_string(&instance.data)
            .map_err(|e| WfeError::Persistence(e.to_string()))?;
        let complete_time_str = opt_dt_to_string(&instance.complete_time);

        let mut tx = self.pool.begin().await.map_err(to_persistence_err)?;

        sqlx::query(
            "UPDATE workflows SET name = ?1, definition_id = ?2, version = ?3, description = ?4, reference = ?5,
             status = ?6, data = ?7, next_execution = ?8, complete_time = ?9
             WHERE id = ?10",
        )
        .bind(&instance.name)
        .bind(&instance.workflow_definition_id)
        .bind(instance.version as i64)
        .bind(&instance.description)
        .bind(&instance.reference)
        .bind(&status_str)
        .bind(&data_str)
        .bind(instance.next_execution)
        .bind(&complete_time_str)
        .bind(&instance.id)
        .execute(&mut *tx)
        .await
        .map_err(to_persistence_err)?;

        // Replace all pointers
        sqlx::query("DELETE FROM execution_pointers WHERE workflow_id = ?1")
            .bind(&instance.id)
            .execute(&mut *tx)
            .await
            .map_err(to_persistence_err)?;

        for ptr in &instance.execution_pointers {
            insert_pointer(&mut tx, &instance.id, ptr).await?;
        }

        tx.commit().await.map_err(to_persistence_err)?;
        Ok(())
    }

    async fn persist_workflow_with_subscriptions(
        &self,
        instance: &WorkflowInstance,
        subscriptions: &[EventSubscription],
    ) -> Result<()> {
        let status_str = serde_json::to_value(instance.status)
            .map_err(|e| WfeError::Persistence(e.to_string()))?
            .as_str()
            .unwrap_or("Runnable")
            .to_string();
        let data_str = serde_json::to_string(&instance.data)
            .map_err(|e| WfeError::Persistence(e.to_string()))?;
        let complete_time_str = opt_dt_to_string(&instance.complete_time);

        let mut tx = self.pool.begin().await.map_err(to_persistence_err)?;

        sqlx::query(
            "UPDATE workflows SET name = ?1, definition_id = ?2, version = ?3, description = ?4, reference = ?5,
             status = ?6, data = ?7, next_execution = ?8, complete_time = ?9
             WHERE id = ?10",
        )
        .bind(&instance.name)
        .bind(&instance.workflow_definition_id)
        .bind(instance.version as i64)
        .bind(&instance.description)
        .bind(&instance.reference)
        .bind(&status_str)
        .bind(&data_str)
        .bind(instance.next_execution)
        .bind(&complete_time_str)
        .bind(&instance.id)
        .execute(&mut *tx)
        .await
        .map_err(to_persistence_err)?;

        sqlx::query("DELETE FROM execution_pointers WHERE workflow_id = ?1")
            .bind(&instance.id)
            .execute(&mut *tx)
            .await
            .map_err(to_persistence_err)?;

        for ptr in &instance.execution_pointers {
            insert_pointer(&mut tx, &instance.id, ptr).await?;
        }

        for sub in subscriptions {
            insert_subscription(&mut tx, sub).await?;
        }

        tx.commit().await.map_err(to_persistence_err)?;
        Ok(())
    }

    async fn get_runnable_instances(&self, as_at: DateTime<Utc>) -> Result<Vec<String>> {
        let as_at_millis = as_at.timestamp_millis();
        let rows = sqlx::query(
            "SELECT id FROM workflows WHERE status = 'Runnable' AND next_execution IS NOT NULL AND next_execution <= ?1",
        )
        .bind(as_at_millis)
        .fetch_all(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        let ids = rows
            .iter()
            .map(|r| r.try_get("id").map_err(to_persistence_err))
            .collect::<Result<Vec<String>>>()?;
        Ok(ids)
    }

    async fn get_workflow_instance(&self, id: &str) -> Result<WorkflowInstance> {
        let row = sqlx::query("SELECT * FROM workflows WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_persistence_err)?
            .ok_or_else(|| WfeError::WorkflowNotFound(id.to_string()))?;

        let pointer_rows = sqlx::query("SELECT * FROM execution_pointers WHERE workflow_id = ?1")
            .bind(id)
            .fetch_all(&self.pool)
            .await
            .map_err(to_persistence_err)?;

        let pointers = pointer_rows
            .iter()
            .map(row_to_pointer)
            .collect::<Result<Vec<ExecutionPointer>>>()?;

        row_to_workflow(&row, pointers)
    }

    async fn get_workflow_instance_by_name(&self, name: &str) -> Result<WorkflowInstance> {
        let row = sqlx::query("SELECT id FROM workflows WHERE name = ?1")
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_persistence_err)?
            .ok_or_else(|| WfeError::WorkflowNotFound(name.to_string()))?;
        let id: String = row.try_get("id").map_err(to_persistence_err)?;
        self.get_workflow_instance(&id).await
    }

    async fn next_definition_sequence(&self, definition_id: &str) -> Result<u64> {
        // SQLite doesn't support `INSERT ... ON CONFLICT ... RETURNING` prior
        // to 3.35, but sqlx bundles a new-enough build. Emulate an atomic
        // increment via UPSERT + RETURNING so concurrent callers don't collide.
        let row = sqlx::query(
            "INSERT INTO definition_sequences (definition_id, next_num)
             VALUES (?1, 1)
             ON CONFLICT(definition_id) DO UPDATE
               SET next_num = next_num + 1
             RETURNING next_num",
        )
        .bind(definition_id)
        .fetch_one(&self.pool)
        .await
        .map_err(to_persistence_err)?;
        let next: i64 = row.try_get("next_num").map_err(to_persistence_err)?;
        Ok(next as u64)
    }

    async fn get_workflow_instances(&self, ids: &[String]) -> Result<Vec<WorkflowInstance>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            match self.get_workflow_instance(id).await {
                Ok(w) => result.push(w),
                Err(WfeError::WorkflowNotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(result)
    }
}

async fn insert_pointer(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    workflow_id: &str,
    ptr: &ExecutionPointer,
) -> Result<()> {
    let status_str = serde_json::to_value(ptr.status)
        .map_err(|e| WfeError::Persistence(e.to_string()))?
        .as_str()
        .unwrap_or("Pending")
        .to_string();
    let persistence_data_str = ptr
        .persistence_data
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| WfeError::Persistence(e.to_string()))?;
    let event_data_str = ptr
        .event_data
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| WfeError::Persistence(e.to_string()))?;
    let context_item_str = ptr
        .context_item
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| WfeError::Persistence(e.to_string()))?;
    let outcome_str = ptr
        .outcome
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| WfeError::Persistence(e.to_string()))?;
    let children_str =
        serde_json::to_string(&ptr.children).map_err(|e| WfeError::Persistence(e.to_string()))?;
    let scope_str =
        serde_json::to_string(&ptr.scope).map_err(|e| WfeError::Persistence(e.to_string()))?;
    let ext_str = serde_json::to_string(&ptr.extension_attributes)
        .map_err(|e| WfeError::Persistence(e.to_string()))?;

    let sleep_until_str = opt_dt_to_string(&ptr.sleep_until);
    let start_time_str = opt_dt_to_string(&ptr.start_time);
    let end_time_str = opt_dt_to_string(&ptr.end_time);

    sqlx::query(
        "INSERT INTO execution_pointers
         (id, workflow_id, step_id, active, status, sleep_until, persistence_data, start_time,
          end_time, event_name, event_key, event_published, event_data, step_name, retry_count,
          children, context_item, predecessor_id, outcome, scope, extension_attributes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
    )
    .bind(&ptr.id)
    .bind(workflow_id)
    .bind(ptr.step_id as i64)
    .bind(ptr.active)
    .bind(&status_str)
    .bind(&sleep_until_str)
    .bind(&persistence_data_str)
    .bind(&start_time_str)
    .bind(&end_time_str)
    .bind(&ptr.event_name)
    .bind(&ptr.event_key)
    .bind(ptr.event_published)
    .bind(&event_data_str)
    .bind(&ptr.step_name)
    .bind(ptr.retry_count as i64)
    .bind(&children_str)
    .bind(&context_item_str)
    .bind(&ptr.predecessor_id)
    .bind(&outcome_str)
    .bind(&scope_str)
    .bind(&ext_str)
    .execute(&mut **tx)
    .await
    .map_err(to_persistence_err)?;

    Ok(())
}

async fn insert_subscription(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    sub: &EventSubscription,
) -> Result<()> {
    let subscribe_as_of_str = dt_to_string(&sub.subscribe_as_of);
    let subscription_data_str = sub
        .subscription_data
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| WfeError::Persistence(e.to_string()))?;
    let external_token_expiry_str = opt_dt_to_string(&sub.external_token_expiry);

    sqlx::query(
        "INSERT INTO event_subscriptions
         (id, workflow_id, step_id, execution_pointer_id, event_name, event_key,
          subscribe_as_of, subscription_data, external_token, external_worker_id,
          external_token_expiry, terminated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0)",
    )
    .bind(&sub.id)
    .bind(&sub.workflow_id)
    .bind(sub.step_id as i64)
    .bind(&sub.execution_pointer_id)
    .bind(&sub.event_name)
    .bind(&sub.event_key)
    .bind(&subscribe_as_of_str)
    .bind(&subscription_data_str)
    .bind(&sub.external_token)
    .bind(&sub.external_worker_id)
    .bind(&external_token_expiry_str)
    .execute(&mut **tx)
    .await
    .map_err(to_persistence_err)?;

    Ok(())
}

#[async_trait]
impl SubscriptionRepository for SqlitePersistenceProvider {
    async fn create_event_subscription(&self, subscription: &EventSubscription) -> Result<String> {
        let id = if subscription.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            subscription.id.clone()
        };

        let mut stored = subscription.clone();
        stored.id = id.clone();

        let mut tx = self.pool.begin().await.map_err(to_persistence_err)?;
        insert_subscription(&mut tx, &stored).await?;
        tx.commit().await.map_err(to_persistence_err)?;
        Ok(id)
    }

    async fn get_subscriptions(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<EventSubscription>> {
        let as_of_str = dt_to_string(&as_of);
        let rows = sqlx::query(
            "SELECT * FROM event_subscriptions
             WHERE event_name = ?1 AND event_key = ?2 AND subscribe_as_of <= ?3 AND terminated = 0",
        )
        .bind(event_name)
        .bind(event_key)
        .bind(&as_of_str)
        .fetch_all(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        rows.iter().map(row_to_subscription).collect()
    }

    async fn terminate_subscription(&self, subscription_id: &str) -> Result<()> {
        let result = sqlx::query("UPDATE event_subscriptions SET terminated = 1 WHERE id = ?1")
            .bind(subscription_id)
            .execute(&self.pool)
            .await
            .map_err(to_persistence_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::SubscriptionNotFound(subscription_id.to_string()));
        }
        Ok(())
    }

    async fn get_subscription(&self, subscription_id: &str) -> Result<EventSubscription> {
        let row = sqlx::query("SELECT * FROM event_subscriptions WHERE id = ?1")
            .bind(subscription_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_persistence_err)?
            .ok_or_else(|| WfeError::SubscriptionNotFound(subscription_id.to_string()))?;

        row_to_subscription(&row)
    }

    async fn get_first_open_subscription(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Option<EventSubscription>> {
        let as_of_str = dt_to_string(&as_of);
        let row = sqlx::query(
            "SELECT * FROM event_subscriptions
             WHERE event_name = ?1 AND event_key = ?2 AND subscribe_as_of <= ?3
               AND terminated = 0 AND external_token IS NULL
             LIMIT 1",
        )
        .bind(event_name)
        .bind(event_key)
        .bind(&as_of_str)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        match row {
            Some(r) => Ok(Some(row_to_subscription(&r)?)),
            None => Ok(None),
        }
    }

    async fn set_subscription_token(
        &self,
        subscription_id: &str,
        token: &str,
        worker_id: &str,
        expiry: DateTime<Utc>,
    ) -> Result<bool> {
        let expiry_str = dt_to_string(&expiry);

        // Only set token if external_token is currently NULL
        let result = sqlx::query(
            "UPDATE event_subscriptions
             SET external_token = ?1, external_worker_id = ?2, external_token_expiry = ?3
             WHERE id = ?4 AND external_token IS NULL",
        )
        .bind(token)
        .bind(worker_id)
        .bind(&expiry_str)
        .bind(subscription_id)
        .execute(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        if result.rows_affected() == 0 {
            // Check if the subscription exists at all
            let exists = sqlx::query("SELECT 1 FROM event_subscriptions WHERE id = ?1")
                .bind(subscription_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(to_persistence_err)?;
            if exists.is_none() {
                return Err(WfeError::SubscriptionNotFound(subscription_id.to_string()));
            }
            return Ok(false);
        }
        Ok(true)
    }

    async fn clear_subscription_token(&self, subscription_id: &str, token: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE event_subscriptions
             SET external_token = NULL, external_worker_id = NULL, external_token_expiry = NULL
             WHERE id = ?1 AND external_token = ?2",
        )
        .bind(subscription_id)
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::SubscriptionNotFound(subscription_id.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl EventRepository for SqlitePersistenceProvider {
    async fn create_event(&self, event: &Event) -> Result<String> {
        let id = if event.id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            event.id.clone()
        };

        let event_data_str = serde_json::to_string(&event.event_data)
            .map_err(|e| WfeError::Persistence(e.to_string()))?;
        let event_time_str = dt_to_string(&event.event_time);

        sqlx::query(
            "INSERT INTO events (id, event_name, event_key, event_data, event_time, is_processed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(&id)
        .bind(&event.event_name)
        .bind(&event.event_key)
        .bind(&event_data_str)
        .bind(&event_time_str)
        .bind(event.is_processed)
        .execute(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        Ok(id)
    }

    async fn get_event(&self, id: &str) -> Result<Event> {
        let row = sqlx::query("SELECT * FROM events WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_persistence_err)?
            .ok_or_else(|| WfeError::EventNotFound(id.to_string()))?;

        row_to_event(&row)
    }

    async fn get_runnable_events(&self, as_at: DateTime<Utc>) -> Result<Vec<String>> {
        let as_at_str = dt_to_string(&as_at);
        let rows = sqlx::query("SELECT id FROM events WHERE is_processed = 0 AND event_time <= ?1")
            .bind(&as_at_str)
            .fetch_all(&self.pool)
            .await
            .map_err(to_persistence_err)?;

        rows.iter()
            .map(|r| r.try_get("id").map_err(to_persistence_err))
            .collect()
    }

    async fn get_events(
        &self,
        event_name: &str,
        event_key: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        let as_of_str = dt_to_string(&as_of);
        let rows = sqlx::query(
            "SELECT id FROM events WHERE event_name = ?1 AND event_key = ?2 AND event_time <= ?3",
        )
        .bind(event_name)
        .bind(event_key)
        .bind(&as_of_str)
        .fetch_all(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        rows.iter()
            .map(|r| r.try_get("id").map_err(to_persistence_err))
            .collect()
    }

    async fn mark_event_processed(&self, id: &str) -> Result<()> {
        let result = sqlx::query("UPDATE events SET is_processed = 1 WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(to_persistence_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::EventNotFound(id.to_string()));
        }
        Ok(())
    }

    async fn mark_event_unprocessed(&self, id: &str) -> Result<()> {
        let result = sqlx::query("UPDATE events SET is_processed = 0 WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(to_persistence_err)?;

        if result.rows_affected() == 0 {
            return Err(WfeError::EventNotFound(id.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl ScheduledCommandRepository for SqlitePersistenceProvider {
    fn supports_scheduled_commands(&self) -> bool {
        true
    }

    async fn schedule_command(&self, command: &ScheduledCommand) -> Result<()> {
        let command_name_str = serde_json::to_value(&command.command_name)
            .map_err(|e| WfeError::Persistence(e.to_string()))?
            .as_str()
            .unwrap_or("")
            .to_string();

        sqlx::query(
            "INSERT OR IGNORE INTO scheduled_commands (command_name, data, execute_time)
             VALUES (?1, ?2, ?3)",
        )
        .bind(&command_name_str)
        .bind(&command.data)
        .bind(command.execute_time)
        .execute(&self.pool)
        .await
        .map_err(to_persistence_err)?;

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

        // Fetch due commands
        let rows = sqlx::query(
            "SELECT id, command_name, data, execute_time FROM scheduled_commands WHERE execute_time <= ?1",
        )
        .bind(as_of_millis)
        .fetch_all(&self.pool)
        .await
        .map_err(to_persistence_err)?;

        let mut commands: Vec<(i64, ScheduledCommand)> = Vec::new();
        for row in &rows {
            let db_id: i64 = row.try_get("id").map_err(to_persistence_err)?;
            let command_name_str: String =
                row.try_get("command_name").map_err(to_persistence_err)?;
            let command_name: CommandName =
                serde_json::from_str(&format!("\"{command_name_str}\"")).map_err(|e| {
                    WfeError::Persistence(format!("Failed to deserialize CommandName: {e}"))
                })?;
            let data: String = row.try_get("data").map_err(to_persistence_err)?;
            let execute_time: i64 = row.try_get("execute_time").map_err(to_persistence_err)?;

            commands.push((
                db_id,
                ScheduledCommand {
                    command_name,
                    data,
                    execute_time,
                },
            ));
        }

        // Process each command then delete it
        for (db_id, cmd) in commands {
            handler(cmd).await?;
            sqlx::query("DELETE FROM scheduled_commands WHERE id = ?1")
                .bind(db_id)
                .execute(&self.pool)
                .await
                .map_err(to_persistence_err)?;
        }
        Ok(())
    }
}

#[async_trait]
impl PersistenceProvider for SqlitePersistenceProvider {
    async fn persist_errors(&self, errors: &[ExecutionError]) -> Result<()> {
        for error in errors {
            let error_time_str = dt_to_string(&error.error_time);
            sqlx::query(
                "INSERT INTO execution_errors (error_time, workflow_id, execution_pointer_id, message)
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(&error_time_str)
            .bind(&error.workflow_id)
            .bind(&error.execution_pointer_id)
            .bind(&error.message)
            .execute(&self.pool)
            .await
            .map_err(to_persistence_err)?;
        }
        Ok(())
    }

    async fn ensure_store_exists(&self) -> Result<()> {
        self.create_tables()
            .await
            .map_err(|e| WfeError::Persistence(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn schema_creation_idempotent() {
        let provider = SqlitePersistenceProvider::new(":memory:").await.unwrap();
        // Call ensure_store_exists again — should not fail
        provider.ensure_store_exists().await.unwrap();
        provider.ensure_store_exists().await.unwrap();
    }
}
