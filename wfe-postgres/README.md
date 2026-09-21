# wfe-postgres

PostgreSQL persistence provider for the WFE workflow engine.

## What it does

Implements the full `PersistenceProvider` trait backed by PostgreSQL via sqlx. All workflow data, events, subscriptions, and scheduled commands live in a dedicated schema (default `wfc`) with a configurable table-name prefix. Uses JSONB for structured data (execution pointer children, scope, extension attributes) and TIMESTAMPTZ for timestamps. Schema and indexes are created automatically via `ensure_store_exists` using standard `sqlx::migrate!` migrations.

## Quick start

```rust
use wfe_postgres::{PostgresOptions, PostgresPersistenceProvider};

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

## Sharing a database with your application

wfe-postgres is built to live inside the **same database** as the application embedding it:

- Every WFE table lives in its own schema (default `wfc`), separate from the application's tables.
- The migration history is recorded in `_sqlx_migrations` **inside that schema too** — because migrations run on a connection whose `search_path` points there. The application's own `_sqlx_migrations` (in `public` or wherever its search_path points) is never read or written, so both sides can run `sqlx::migrate!` against the same database independently.

Pick a different schema, and optionally a table-name prefix:

```rust
let provider = PostgresPersistenceProvider::connect(
    "postgres://app:app@localhost:5432/app_db",
    PostgresOptions {
        schema: "myapp_wfe".into(),   // default: "wfc"
        table_prefix: String::new(),  // default: no prefix
    },
).await?;
provider.ensure_store_exists().await?;
```

- **schema**: created automatically if missing. If the database user may not create schemas, have your DBA pre-create it; `ensure_store_exists` then skips that step.
- **table_prefix**: for hosts that require WFE tables to live in a shared schema (e.g. `public`) under distinct names — `table_prefix: "wfe_"` produces `public.wfe_workflows`, `public.wfe_events`, and so on. In this mode migrations run through the same embedded `sqlx::migrate!` set with identifiers rewritten at execution time, and progress is recorded in a separate `_wfe_sqlx_migrations` table so it cannot mix with the application's history. Schema and prefix are validated against PostgreSQL identifier rules; both must match `[A-Za-z_][A-Za-z0-9_]*`. The prefix of an existing store cannot be changed in place — checkpoints are keyed to the rewritten SQL, so a change is reported as an error rather than silently stranding the tracking table.

`PostgresPersistenceProvider::from_pool` / `from_pool_with` build the provider on an existing pool, for tests or shared pool configuration. The pool's own `search_path` is irrelevant: every runtime query is schema-qualified.

## wfe-server configuration

```toml
[persistence]
backend = "postgres"
url = "postgres://user:pass@host:5432/db"
schema = "wfc"          # optional
table_prefix = ""       # optional
```

Equivalent CLI flags / environment variables: `--db-schema` / `WFE_DB_SCHEMA` and `--db-table-prefix` / `WFE_DB_TABLE_PREFIX`. Overrides preserve values the config file set when the CLI does not restate them.

## API

| Type | Trait |
|------|-------|
| `PostgresPersistenceProvider` | `PersistenceProvider`, `WorkflowRepository`, `EventRepository`, `SubscriptionRepository`, `ScheduledCommandRepository` |
| `PostgresOptions` | — (`schema`, `table_prefix`) |

Constructors:

- `new(url)` — default layout (schema `wfc`, no prefix)
- `connect(url, options)` — explicit layout
- `from_pool(pool)` / `from_pool_with(pool, options)` — wrap an existing pool

Additional methods:

- `truncate_all()` — truncates all tables with CASCADE, useful for test cleanup
- `options()` — the layout this provider was built with

## Configuration

Connection string follows the standard PostgreSQL URI format:

```
postgres://user:password@host:port/database
```

The pool is configured with up to 10 connections.

## Schema

Tables created in the configured schema (shown with the default `wfc` and a `wfe_` prefix example):

| Table | Purpose |
|-------|---------|
| `wfc.workflows` / `public.wfe_workflows` | Workflow instances. `data` is JSONB, timestamps are TIMESTAMPTZ. |
| `wfc.execution_pointers` | Step state. `children`, `scope`, `extension_attributes` are JSONB. References `workflows(id)`. |
| `wfc.events` | Published events. `event_data` is JSONB. |
| `wfc.event_subscriptions` | Active subscriptions with CAS-style external token locking. |
| `wfc.scheduled_commands` | Deferred commands. Unique on `(command_name, data)` with upsert semantics. |
| `wfc.execution_errors` | Error log with auto-incrementing serial primary key. |
| `wfc._sqlx_migrations` / `public._wfe_sqlx_migrations` | Migration tracking (stock sqlx table; prefixed mode uses its own). |

Indexes are created on `next_execution`, `status`, `(event_name, event_key)`, `is_processed`, `event_time`, `workflow_id`, and `execute_time`.

## Testing

Requires a running PostgreSQL instance. The default test connection string is `postgres://wfe:wfe@localhost:5432/wfe_test`; override it with `WFE_PG_TEST_URL` when port 5432 is occupied by another PostgreSQL (for example a native install shadowing a Docker-published one):

```sh
docker run -d --name wfe-pg -e POSTGRES_USER=wfe -e POSTGRES_PASSWORD=wfe \
  -e POSTGRES_DB=wfe_test -p 5432:5432 postgres:16
cargo nextest run -p wfe-postgres
```

The suite must run serialized (shared database state); the repository's nextest config in `.config/nextest.toml` already does this, so prefer `cargo nextest run` over plain `cargo test`.

## License

MIT
