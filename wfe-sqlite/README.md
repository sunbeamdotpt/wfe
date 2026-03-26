# wfe-sqlite

SQLite persistence provider for the WFE workflow engine.

## What it does

Implements the full `PersistenceProvider` trait (workflows, events, subscriptions, scheduled commands, execution errors) backed by SQLite via sqlx. Supports both in-memory and file-based databases. Tables and indexes are created automatically on startup -- no migrations to run.

## Quick start

```rust
use wfe_sqlite::SqlitePersistenceProvider;

// In-memory (good for tests and single-process deployments)
let provider = SqlitePersistenceProvider::new(":memory:").await?;

// File-based (persistent across restarts)
let provider = SqlitePersistenceProvider::new("sqlite:///tmp/wfe.db").await?;
```

Wire it into the WFE host as your persistence layer:

```rust
let host = WorkflowHost::new(provider);
```

## API

| Type | Trait |
|------|-------|
| `SqlitePersistenceProvider` | `PersistenceProvider`, `WorkflowRepository`, `EventRepository`, `SubscriptionRepository`, `ScheduledCommandRepository` |

Key methods come from the traits -- `create_new_workflow`, `persist_workflow`, `get_runnable_instances`, `create_event`, `schedule_command`, and so on. See `wfe-core` for the full trait definitions.

## Configuration

The constructor takes a standard SQLite connection string:

| Value | Behavior |
|-------|----------|
| `":memory:"` | In-memory database, single connection, lost on drop |
| `"sqlite:///path/to/db"` | File-backed, WAL journal mode, up to 4 connections |

WAL mode and foreign keys are enabled automatically. For in-memory mode, the pool is capped at 1 connection to avoid separate database instances.

## Schema

Six tables are created in the default schema:

- `workflows` -- workflow instance state and metadata
- `execution_pointers` -- step execution state, linked to workflows via foreign key
- `events` -- published events pending processing
- `event_subscriptions` -- active event subscriptions with CAS-style token locking
- `scheduled_commands` -- deferred commands with dedup on `(command_name, data)`
- `execution_errors` -- error log for failed step executions

## Testing

No external dependencies required. Tests run against `:memory:`:

```sh
cargo test -p wfe-sqlite
```

## License

MIT
