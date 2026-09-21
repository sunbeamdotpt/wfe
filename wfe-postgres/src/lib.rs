//! wfe-postgres — PostgreSQL persistence provider for the WFE workflow engine.
use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha384};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, PgPool, Row};

use wfe_core::models::{
    CommandName, Event, EventSubscription, ExecutionError, ExecutionPointer, PointerStatus,
    ScheduledCommand, WorkflowInstance, WorkflowStatus,
};
use wfe_core::traits::{
    EventRepository, PersistenceProvider, ScheduledCommandRepository, SubscriptionRepository,
    WorkflowRepository,
};
use wfe_core::{Result, WfeError};

/// Postgres NAMEDATALEN limit: no identifier can exceed 63 bytes.
const MAX_IDENTIFIER_LEN: usize = 63;

/// Identifiers created or referenced by wfe-postgres migrations. Table-prefix
/// mode rewrites exactly these, so every table and index name that appears in a
/// migration file must be listed here. The
/// `every_migration_identifier_is_prefixable` test fails the build if a
/// migration names something that is missing.
const PREFIXABLE_IDENTIFIERS: &[&str] = &[
    // tables
    "workflows",
    "definition_sequences",
    "execution_pointers",
    "events",
    "event_subscriptions",
    "execution_errors",
    "scheduled_commands",
    // indexes
    "idx_workflows_next_execution",
    "idx_workflows_status",
    "idx_events_name_key",
    "idx_events_is_processed",
    "idx_events_event_time",
    "idx_subscriptions_name_key",
    "idx_subscriptions_workflow",
    "idx_scheduled_commands_execute_time",
];

/// Tracking table for table-prefix mode. Kept distinct from sqlx's
/// `_sqlx_migrations` so a prefixed store can live in a schema the host
/// application also migrates (e.g. `public`) without mixing histories.
const PREFIXED_MIGRATIONS_TABLE: &str = "_wfe_sqlx_migrations";

/// Advisory-lock id that serializes concurrent wfe nodes booting in prefix
/// mode. Deliberately distinct from sqlx's per-database migration lock so the
/// host application's own migrator is never blocked by wfe (and vice versa).
const PREFIXED_MIGRATION_LOCK_ID: i64 = 0x7766_656D_6967_7252;

/// Where and how wfe-postgres lays out its tables inside the connected
/// database.
///
/// The provider always creates its own schema (default `wfc`) and keeps every
/// table — including sqlx's migration tracking table — inside it, so it can
/// share a database with an application that runs its own sqlx migrations
/// against `public`. Set [`PostgresOptions::table_prefix`] to instead keep the
/// unprefixed schema but rename tables (e.g. `wfe_workflows`), for hosts that
/// require everything in a shared schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresOptions {
    /// Schema that holds all wfe tables. Created automatically if missing
    /// (requires `CREATE` on the database; otherwise have your DBA pre-create
    /// it — `ensure_store_exists` is then a no-op for the schema step).
    pub schema: String,
    /// Prefix prepended to every table and index name, e.g. `wfe_` produces
    /// `wfe_workflows`. Empty by default.
    pub table_prefix: String,
}

impl Default for PostgresOptions {
    fn default() -> Self {
        Self {
            schema: "wfc".to_string(),
            table_prefix: String::new(),
        }
    }
}

impl PostgresOptions {
    /// Validate identifiers the way PostgreSQL will see them: bare, unquoted
    /// names consisting of ASCII letters, digits, and underscores.
    fn validate(&self) -> std::result::Result<(), sqlx::Error> {
        let invalid = |what: &str, value: &str| {
            sqlx::Error::Configuration(
                format!(
                    "invalid {what} {value:?}: must match [A-Za-z_][A-Za-z0-9_]* and contain no \
                     quoting or punctuation"
                )
                .into(),
            )
        };

        let check =
            |what: &str, value: &str, allow_empty: bool| -> std::result::Result<(), sqlx::Error> {
                if value.is_empty() {
                    if allow_empty {
                        return Ok(());
                    }
                    return Err(invalid(what, value));
                }
                let mut chars = value.chars();
                let first_ok = chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                let rest_ok = value
                    .chars()
                    .skip(1)
                    .all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !first_ok || !rest_ok {
                    return Err(invalid(what, value));
                }
                Ok(())
            };

        check("schema", &self.schema, false)?;
        if self.schema.len() > MAX_IDENTIFIER_LEN {
            return Err(sqlx::Error::Configuration(
                format!(
                    "schema {:?} exceeds PostgreSQL's {MAX_IDENTIFIER_LEN}-byte identifier limit",
                    self.schema
                )
                .into(),
            ));
        }

        check("table prefix", &self.table_prefix, true)?;
        // The longest base name plus the prefix must still fit NAMEDATALEN.
        let longest_base = PREFIXABLE_IDENTIFIERS
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0);
        if self.table_prefix.len() + longest_base > MAX_IDENTIFIER_LEN {
            return Err(sqlx::Error::Configuration(
                format!(
                    "table prefix {:?} is too long: with the longest table name it exceeds \
                     PostgreSQL's {MAX_IDENTIFIER_LEN}-byte identifier limit",
                    self.table_prefix
                )
                .into(),
            ));
        }
        Ok(())
    }
}

/// Fully-qualified table names derived from a [`PostgresOptions`], built once
/// at construction and interpolated into every query.
#[derive(Debug, Clone)]
struct QualifiedTables {
    workflows: String,
    definition_sequences: String,
    execution_pointers: String,
    events: String,
    event_subscriptions: String,
    execution_errors: String,
    scheduled_commands: String,
}

impl QualifiedTables {
    fn new(options: &PostgresOptions) -> Self {
        let qualify = |base: &str| {
            format!(
                "\"{}\".\"{}{}\"",
                options.schema, options.table_prefix, base
            )
        };
        Self {
            workflows: qualify("workflows"),
            definition_sequences: qualify("definition_sequences"),
            execution_pointers: qualify("execution_pointers"),
            events: qualify("events"),
            event_subscriptions: qualify("event_subscriptions"),
            execution_errors: qualify("execution_errors"),
            scheduled_commands: qualify("scheduled_commands"),
        }
    }
}

/// PostgreSQL-backed persistence provider for the WFE workflow engine.
///
/// Stores workflows, execution pointers, events, subscriptions, errors, and
/// scheduled commands in PostgreSQL tables inside a dedicated schema
/// (configurable via [`PostgresOptions`], default `wfc`). The schema, tables,
/// and indexes are created automatically on first use; see
/// [`PostgresPersistenceProvider::connect`] for coexisting with an application
/// that owns the same database.
pub struct PostgresPersistenceProvider {
    pool: PgPool,
    options: PostgresOptions,
    tables: QualifiedTables,
}

impl PostgresPersistenceProvider {
    /// Connect using the default layout: schema `wfc`, no table prefix.
    pub async fn new(database_url: &str) -> std::result::Result<Self, sqlx::Error> {
        Self::connect(database_url, PostgresOptions::default()).await
    }

    /// Connect with an explicit [`PostgresOptions`].
    pub async fn connect(
        database_url: &str,
        options: PostgresOptions,
    ) -> std::result::Result<Self, sqlx::Error> {
        options.validate()?;
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        Self::from_pool_with(pool, options)
    }

    /// Wrap an existing pool with the default layout (schema `wfc`, no table
    /// prefix). Useful for tests with custom pool options.
    pub fn from_pool(pool: PgPool) -> Self {
        Self::from_pool_with(pool, PostgresOptions::default())
            .expect("default options are always valid")
    }

    /// Wrap an existing pool with an explicit [`PostgresOptions`].
    pub fn from_pool_with(
        pool: PgPool,
        options: PostgresOptions,
    ) -> std::result::Result<Self, sqlx::Error> {
        options.validate()?;
        let tables = QualifiedTables::new(&options);
        Ok(Self {
            pool,
            options,
            tables,
        })
    }

    /// The options this provider was built with.
    pub fn options(&self) -> &PostgresOptions {
        &self.options
    }

    /// Truncate all tables (for test cleanup).
    pub async fn truncate_all(&self) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(&format!(
            "TRUNCATE {}, {}, {}, {}, {}, {} CASCADE",
            self.tables.execution_errors,
            self.tables.execution_pointers,
            self.tables.event_subscriptions,
            self.tables.events,
            self.tables.scheduled_commands,
            self.tables.workflows,
        ))
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

            sqlx::query(&format!(
                r#"INSERT INTO {}
                   (id, workflow_id, step_id, active, status, sleep_until,
                    persistence_data, start_time, end_time, event_name, event_key,
                    event_published, event_data, step_name, retry_count, children,
                    context_item, predecessor_id, outcome, scope, extension_attributes)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)"#,
                self.tables.execution_pointers,
            ))
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
        let rows = sqlx::query(&format!(
            "SELECT * FROM {} WHERE workflow_id = $1",
            self.tables.execution_pointers
        ))
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

    /// Apply the embedded `sqlx::migrate!` migrations with every
    /// [`PREFIXABLE_IDENTIFIERS`] name rewritten to carry `prefix`, recording
    /// progress in [`PREFIXED_MIGRATIONS_TABLE`] using the same row shape and
    /// dirty-flag protocol as sqlx itself.
    ///
    /// `search_path` must already point at the target schema. Checksums cover
    /// the *rewritten* SQL, so changing the prefix of an existing store is
    /// reported as a checksum mismatch instead of silently stranding the
    /// tracking table away from the tables it describes.
    async fn run_prefixed_migrations(conn: &mut sqlx::PgConnection, prefix: &str) -> Result<()> {
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(PREFIXED_MIGRATION_LOCK_ID)
            .execute(&mut *conn)
            .await
            .map_err(Self::map_sqlx_err)?;

        let result = Self::apply_prefixed_migrations_locked(conn, prefix).await;

        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(PREFIXED_MIGRATION_LOCK_ID)
            .execute(&mut *conn)
            .await;

        result
    }

    async fn apply_prefixed_migrations_locked(
        conn: &mut sqlx::PgConnection,
        prefix: &str,
    ) -> Result<()> {
        // Same shape as sqlx's _sqlx_migrations.
        sqlx::query(&format!(
            r#"CREATE TABLE IF NOT EXISTS {PREFIXED_MIGRATIONS_TABLE} (
               version BIGINT PRIMARY KEY,
               description TEXT NOT NULL,
               installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
               success BOOLEAN NOT NULL,
               checksum BYTEA NOT NULL,
               execution_time BIGINT NOT NULL
           )"#
        ))
        .execute(&mut *conn)
        .await
        .map_err(Self::map_sqlx_err)?;

        let dirty: Option<(i64,)> = sqlx::query_as(&format!(
            "SELECT version FROM {PREFIXED_MIGRATIONS_TABLE} WHERE success = FALSE ORDER BY version LIMIT 1"
        ))
        .fetch_optional(&mut *conn)
        .await
        .map_err(Self::map_sqlx_err)?;
        if let Some((version,)) = dirty {
            return Err(WfeError::Persistence(format!(
                "migration {version} previously failed while applying (dirty); resolve it manually \
                 and clear the row in {PREFIXED_MIGRATIONS_TABLE}"
            )));
        }

        let applied: Vec<(i64, Vec<u8>)> = sqlx::query_as(&format!(
            "SELECT version, checksum FROM {PREFIXED_MIGRATIONS_TABLE} ORDER BY version"
        ))
        .fetch_all(&mut *conn)
        .await
        .map_err(Self::map_sqlx_err)?;
        let applied: HashMap<i64, Vec<u8>> = applied.into_iter().collect();

        for migration in sqlx::migrate!("./migrations").iter() {
            if migration.migration_type.is_down_migration() {
                continue;
            }

            let sql = substitute_identifiers(&migration.sql, prefix);
            let checksum = Sha384::digest(sql.as_bytes()).to_vec();

            match applied.get(&migration.version) {
                Some(recorded) if *recorded == checksum => continue,
                Some(_) => {
                    return Err(WfeError::Persistence(format!(
                        "migration {} was previously applied with a different table prefix; the \
                         prefix of an existing store cannot be changed in place",
                        migration.version
                    )));
                }
                None => {
                    let mut tx = conn.begin().await.map_err(Self::map_sqlx_err)?;
                    // Simple query protocol (not a prepared statement): a
                    // migration file is a multi-statement script. Called via
                    // `Executor::execute` on the connection because
                    // `RawQuery::execute` trips a higher-ranked-trait-bound
                    // check across the async_trait boxing.
                    sqlx::Executor::execute(&mut *tx, sql.as_str())
                        .await
                        .map_err(Self::map_sqlx_err)?;
                    sqlx::query(&format!(
                        r#"INSERT INTO {PREFIXED_MIGRATIONS_TABLE}
                           (version, description, success, checksum, execution_time)
                           VALUES ($1, $2, TRUE, $3, -1)"#
                    ))
                    .bind(migration.version)
                    .bind(&*migration.description)
                    .bind(&checksum)
                    .execute(&mut *tx)
                    .await
                    .map_err(Self::map_sqlx_err)?;
                    tx.commit().await.map_err(Self::map_sqlx_err)?;
                }
            }
        }
        Ok(())
    }
}

/// Rewrite every occurrence of a [`PREFIXABLE_IDENTIFIERS`] name in `sql`,
/// leaving string literals (`'...'` with `''` escapes), `--` line comments,
/// and `/* */` block comments untouched. The migrations are authored with
/// unqualified names, so nothing else needs rewriting.
///
/// Dollar-quoted strings (`$$...$$`) are not supported; do not use them in
/// migration files.
fn substitute_identifiers(sql: &str, prefix: &str) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len() + prefix.len() * 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        out.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            out.push('/');
            out.push('*');
            i += 2;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push(chars[i + 1]);
                    i += 2;
                    break;
                }
                i += 1;
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            if PREFIXABLE_IDENTIFIERS.contains(&ident.as_str()) {
                out.push_str(prefix);
            }
            out.push_str(&ident);
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
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

        sqlx::query(&format!(
            r#"INSERT INTO {}
               (id, name, root_workflow_id, definition_id, version, description, reference,
                status, data, next_execution, create_time, complete_time)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)"#,
            self.tables.workflows,
        ))
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

        sqlx::query(&format!(
            r#"UPDATE {} SET
               name=$2, root_workflow_id=$3, definition_id=$4, version=$5,
               description=$6, reference=$7, status=$8, data=$9, next_execution=$10,
               create_time=$11, complete_time=$12,
               last_heartbeat_at=now()
               WHERE id=$1"#,
            self.tables.workflows,
        ))
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
        sqlx::query(&format!(
            "DELETE FROM {} WHERE workflow_id = $1",
            self.tables.execution_pointers
        ))
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

        sqlx::query(&format!(
            r#"UPDATE {} SET
               name=$2, root_workflow_id=$3, definition_id=$4, version=$5,
               description=$6, reference=$7, status=$8, data=$9, next_execution=$10,
               create_time=$11, complete_time=$12,
               last_heartbeat_at=now()
               WHERE id=$1"#,
            self.tables.workflows,
        ))
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
        sqlx::query(&format!(
            "DELETE FROM {} WHERE workflow_id = $1",
            self.tables.execution_pointers
        ))
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
            sqlx::query(&format!(
                r#"INSERT INTO {}
                   (id, workflow_id, step_id, execution_pointer_id, event_name, event_key,
                    subscribe_as_of, subscription_data, external_token, external_worker_id,
                    external_token_expiry)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
                self.tables.event_subscriptions,
            ))
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
        let rows = sqlx::query(&format!(
            "SELECT id FROM {} WHERE status = 'Runnable' AND next_execution <= $1",
            self.tables.workflows
        ))
        .bind(as_at_millis)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    async fn get_workflow_instance(&self, id: &str) -> Result<WorkflowInstance> {
        let row = sqlx::query(&format!(
            "SELECT * FROM {} WHERE id = $1",
            self.tables.workflows
        ))
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
        let row = sqlx::query(&format!(
            "SELECT id FROM {} WHERE name = $1",
            self.tables.workflows
        ))
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
        let row = sqlx::query(&format!(
            r#"INSERT INTO {} (definition_id, next_num)
               VALUES ($1, 1)
               ON CONFLICT (definition_id) DO UPDATE
                 SET next_num = {}.next_num + 1
               RETURNING next_num"#,
            self.tables.definition_sequences, self.tables.definition_sequences,
        ))
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

        sqlx::query(&format!(
            r#"INSERT INTO {}
               (id, workflow_id, step_id, execution_pointer_id, event_name, event_key,
                subscribe_as_of, subscription_data, external_token, external_worker_id,
                external_token_expiry)
               VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"#,
            self.tables.event_subscriptions,
        ))
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
        let rows = sqlx::query(&format!(
            r#"SELECT * FROM {}
               WHERE event_name = $1 AND event_key = $2
                 AND subscribe_as_of <= $3
                 AND external_token IS NULL"#,
            self.tables.event_subscriptions,
        ))
        .bind(event_name)
        .bind(event_key)
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| self.row_to_subscription(r)).collect())
    }

    async fn terminate_subscription(&self, subscription_id: &str) -> Result<()> {
        let result = sqlx::query(&format!(
            "DELETE FROM {} WHERE id = $1",
            self.tables.event_subscriptions
        ))
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
        let row = sqlx::query(&format!(
            "SELECT * FROM {} WHERE id = $1",
            self.tables.event_subscriptions
        ))
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
        let row = sqlx::query(&format!(
            r#"SELECT * FROM {}
               WHERE event_name = $1 AND event_key = $2
                 AND subscribe_as_of <= $3
                 AND external_token IS NULL
               LIMIT 1"#,
            self.tables.event_subscriptions,
        ))
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
        let result = sqlx::query(&format!(
            r#"UPDATE {}
               SET external_token = $2, external_worker_id = $3, external_token_expiry = $4
               WHERE id = $1 AND external_token IS NULL"#,
            self.tables.event_subscriptions,
        ))
        .bind(subscription_id)
        .bind(token)
        .bind(worker_id)
        .bind(expiry)
        .execute(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        if result.rows_affected() == 0 {
            // Check if subscription exists
            let exists = sqlx::query(&format!(
                "SELECT 1 FROM {} WHERE id = $1",
                self.tables.event_subscriptions
            ))
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
        let result = sqlx::query(&format!(
            r#"UPDATE {}
               SET external_token = NULL, external_worker_id = NULL, external_token_expiry = NULL
               WHERE id = $1 AND external_token = $2"#,
            self.tables.event_subscriptions,
        ))
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

        sqlx::query(&format!(
            r#"INSERT INTO {}
               (id, event_name, event_key, event_data, event_time, is_processed)
               VALUES ($1,$2,$3,$4,$5,$6)"#,
            self.tables.events,
        ))
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
        let row = sqlx::query(&format!(
            "SELECT * FROM {} WHERE id = $1",
            self.tables.events
        ))
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
        let rows = sqlx::query(&format!(
            "SELECT id FROM {} WHERE is_processed = FALSE AND event_time <= $1",
            self.tables.events
        ))
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
        let rows = sqlx::query(&format!(
            r#"SELECT id FROM {}
               WHERE event_name = $1 AND event_key = $2 AND event_time <= $3"#,
            self.tables.events,
        ))
        .bind(event_name)
        .bind(event_key)
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(Self::map_sqlx_err)?;

        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    async fn mark_event_processed(&self, id: &str) -> Result<()> {
        let result = sqlx::query(&format!(
            "UPDATE {} SET is_processed = TRUE WHERE id = $1",
            self.tables.events
        ))
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
        let result = sqlx::query(&format!(
            "UPDATE {} SET is_processed = FALSE WHERE id = $1",
            self.tables.events
        ))
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
        sqlx::query(&format!(
            r#"INSERT INTO {} (command_name, data, execute_time)
               VALUES ($1, $2, $3)
               ON CONFLICT (command_name, data) DO UPDATE SET execute_time = $3"#,
            self.tables.scheduled_commands,
        ))
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
        let rows = sqlx::query(&format!(
            "SELECT * FROM {} WHERE execute_time <= $1",
            self.tables.scheduled_commands
        ))
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
        sqlx::query(&format!(
            "DELETE FROM {} WHERE execute_time <= $1",
            self.tables.scheduled_commands
        ))
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
            sqlx::query(&format!(
                r#"INSERT INTO {}
                   (error_time, workflow_id, execution_pointer_id, message)
                   VALUES ($1,$2,$3,$4)"#,
                self.tables.execution_errors,
            ))
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
        // The schema is created out-of-band so the migration files themselves
        // can stay unqualified and resolve via search_path. If this fails on
        // permissions, the schema can be pre-created by a DBA instead.
        sqlx::query(&format!(
            "CREATE SCHEMA IF NOT EXISTS \"{}\"",
            self.options.schema
        ))
        .execute(&self.pool)
        .await
        .map_err(|e| {
            WfeError::Persistence(format!(
                "failed to create schema {:?}: {e}. If the database user may not create \
                 schemas, have your DBA pre-create {:?} and retry",
                self.options.schema, self.options.schema
            ))
        })?;

        // Migrations run on a dedicated connection with search_path pointed at
        // the target schema. sqlx's tracking table is created unqualified, so
        // it lands inside that schema and never collides with a host
        // application's own _sqlx_migrations (which lives wherever the app's
        // search_path points, usually public).
        let mut conn = self.pool.acquire().await.map_err(Self::map_sqlx_err)?;
        sqlx::query(&format!("SET search_path TO \"{}\"", self.options.schema))
            .execute(&mut *conn)
            .await
            .map_err(Self::map_sqlx_err)?;

        if self.options.table_prefix.is_empty() {
            // `run_direct` rather than `run`: `&mut PgConnection` trips the
            // "implementation of `Acquire` is not general enough" error through
            // `run`, which is precisely why sqlx exposes `run_direct`.
            sqlx::migrate!("./migrations")
                .run_direct(&mut *conn)
                .await
                .map_err(|e| {
                    WfeError::Persistence(format!(
                        "migration failed in schema {:?}: {e}",
                        self.options.schema
                    ))
                })?;
        } else {
            Self::run_prefixed_migrations(&mut conn, &self.options.table_prefix).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_valid() {
        PostgresOptions::default().validate().unwrap();
    }

    #[test]
    fn accepts_custom_schema_and_prefix() {
        PostgresOptions {
            schema: "wfe_prod".into(),
            table_prefix: "wfe_".into(),
        }
        .validate()
        .unwrap();
    }

    #[test]
    fn rejects_hostile_or_malformed_schema() {
        for schema in [
            "",
            "wfc; DROP TABLE x",
            "wfc--comment",
            "1wfc",
            "has space",
            "public.wfc",
            "\"quoted\"",
            &"x".repeat(64),
        ] {
            let err = PostgresOptions {
                schema: schema.into(),
                table_prefix: String::new(),
            }
            .validate();
            assert!(err.is_err(), "schema {schema:?} should be rejected");
        }
    }

    #[test]
    fn rejects_malformed_table_prefix() {
        for prefix in ["1abc", "a-b", "a.b", "a b", "abc'"] {
            let err = PostgresOptions {
                schema: "wfc".into(),
                table_prefix: prefix.into(),
            }
            .validate();
            assert!(err.is_err(), "prefix {prefix:?} should be rejected");
        }
    }

    #[test]
    fn rejects_prefix_that_overflows_identifier_limit() {
        let err = PostgresOptions {
            schema: "wfc".into(),
            table_prefix: "w".repeat(MAX_IDENTIFIER_LEN),
        }
        .validate();
        assert!(err.is_err());
    }

    #[test]
    fn substitution_rewrites_known_identifiers_only() {
        let sql = "\
-- the workflows table (comment mentions workflows)
CREATE TABLE IF NOT EXISTS workflows (name TEXT DEFAULT 'workflows');
CREATE INDEX IF NOT EXISTS idx_workflows_status ON workflows (status);
/* events in a block comment */
INSERT INTO events (id) VALUES ('event_subscriptions literal');";
        let out = substitute_identifiers(sql, "wfe_");
        assert!(out.contains("CREATE TABLE IF NOT EXISTS wfe_workflows"));
        assert!(out.contains("DEFAULT 'workflows'"));
        assert!(
            out.contains("CREATE INDEX IF NOT EXISTS wfe_idx_workflows_status ON wfe_workflows")
        );
        assert!(out.contains("INSERT INTO wfe_events"));
        assert!(out.contains("'event_subscriptions literal'"));
        assert!(!out.contains(" wfe_wfe_"));
    }

    #[test]
    fn substitution_with_empty_prefix_is_identity() {
        let sql = "CREATE TABLE IF NOT EXISTS workflows (id TEXT);";
        assert_eq!(substitute_identifiers(sql, ""), sql);
    }

    /// Fails when a migration file creates or references a table/index that is
    /// missing from [`PREFIXABLE_IDENTIFIERS`], so prefix mode can never
    /// silently skip a new object.
    #[test]
    fn every_migration_identifier_is_prefixable() {
        for migration in sqlx::migrate!("./migrations").iter() {
            for ident in migration_created_or_referenced_identifiers(&migration.sql) {
                assert!(
                    PREFIXABLE_IDENTIFIERS.contains(&ident.as_str()),
                    "migration {} references `{ident}` which is not listed in \
                     PREFIXABLE_IDENTIFIERS; add it (and keep migration files unqualified)",
                    migration.version
                );
            }
        }
    }

    /// Extracts the identifier following TABLE / INDEX / REFERENCES / ON,
    /// skipping IF NOT EXISTS, ignoring literals and comments. `ON CONFLICT`
    /// is excluded because it is not a table reference.
    fn migration_created_or_referenced_identifiers(sql: &str) -> Vec<String> {
        let chars: Vec<char> = sql.chars().collect();
        let mut words: Vec<String> = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '\'' {
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\'' {
                        if chars.get(i + 1) == Some(&'\'') {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            } else if c == '-' && chars.get(i + 1) == Some(&'-') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && chars.get(i + 1) == Some(&'*') {
                i += 2;
                while i < chars.len() {
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                words.push(chars[start..i].iter().collect());
            } else {
                i += 1;
            }
        }

        let mut out = Vec::new();
        let mut w = 0;
        while w < words.len() {
            if matches!(
                words[w].as_str(),
                "TABLE" | "table" | "INDEX" | "index" | "REFERENCES" | "references" | "ON" | "on"
            ) {
                let mut j = w + 1;
                while j < words.len()
                    && matches!(
                        words[j].as_str(),
                        "IF" | "if" | "NOT" | "not" | "EXISTS" | "exists"
                    )
                {
                    j += 1;
                }
                if let Some(next) = words.get(j) {
                    // `ON CONFLICT` is control flow, not a table reference.
                    let is_on_conflict = (words[w] == "ON" || words[w] == "on")
                        && (next == "CONFLICT" || next == "conflict");
                    if !is_on_conflict {
                        out.push(next.clone());
                    }
                }
            }
            w += 1;
        }
        out
    }
}
