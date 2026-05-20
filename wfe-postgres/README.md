# wfe-postgres

PostgreSQL persistence provider for the WFE workflow engine.

## What it does

Implements the full `PersistenceProvider` trait backed by PostgreSQL via sqlx. All workflow data, events, subscriptions, and scheduled commands live in a dedicated `wfc` schema. Uses JSONB for structured data (execution pointer children, scope, extension attributes) and TIMESTAMPTZ for timestamps. Schema and indexes are created automatically via `ensure_store_exists`.

## Quick start

```rust
use wfe_postgres::PostgresPersistenceProvider;

let provider = PostgresPersistenceProvider::new(
    "postgres://wfe:wfe@localhost:5432/wfe_test"
).await?;

// Create schema and tables (idempotent)
provider.ensure_store_exists().await?;
```

Wire it into the WFE host:

```rust
let host = WorkflowHost::new(provider);
```

## API

| Type | Trait |
|------|-------|
| `PostgresPersistenceProvider` | `PersistenceProvider`, `WorkflowRepository`, `EventRepository`, `SubscriptionRepository`, `ScheduledCommandRepository` |

Additional methods:

- `truncate_all()` -- truncates all tables with CASCADE, useful for test cleanup

## Configuration

Connection string follows the standard PostgreSQL URI format:

```
postgres://user:password@host:port/database
```

The pool is configured with up to 10 connections. All tables are created under the `wfc` schema.

## Schema

Tables created in `wfc`:

| Table | Purpose |
|-------|---------|
| `wfc.workflows` | Workflow instances. `data` is JSONB, timestamps are TIMESTAMPTZ. |
| `wfc.execution_pointers` | Step state. `children`, `scope`, `extension_attributes` are JSONB. References `wfc.workflows(id)`. |
| `wfc.events` | Published events. `event_data` is JSONB. |
| `wfc.event_subscriptions` | Active subscriptions with CAS-style external token locking. |
| `wfc.scheduled_commands` | Deferred commands. Unique on `(command_name, data)` with upsert semantics. |
| `wfc.execution_errors` | Error log with auto-incrementing serial primary key. |

Indexes are created on `next_execution`, `status`, `(event_name, event_key)`, `is_processed`, `event_time`, `workflow_id`, and `execute_time`.

## Testing

Requires a running PostgreSQL instance. Use the project docker-compose:

```sh
docker compose up -d postgres
cargo test -p wfe-postgres
```

Default test connection string: `postgres://wfe:wfe@localhost:5432/wfe_test`

## License

MIT
