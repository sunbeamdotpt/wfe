# wfe-opensearch

OpenSearch search index provider for the WFE workflow engine.

## What it does

Implements the `SearchIndex` trait backed by OpenSearch. Indexes workflow instances as documents and supports full-text search with bool queries, term filters (status, reference), date range filters, and pagination. The index mapping is created automatically on `start()` if it does not already exist.

## Quick start

```rust
use wfe_opensearch::OpenSearchIndex;

let index = OpenSearchIndex::new(
    "http://localhost:9200",
    "wfe_workflows",
)?;

// Create the index mapping (idempotent)
index.start().await?;

// Index a workflow
index.index_workflow(&workflow_instance).await?;

// Search with filters
use wfe_core::traits::search::{SearchFilter, SearchIndex};

let page = index.search(
    "deploy",         // free-text query
    0,                // skip
    20,               // take
    &[SearchFilter::Status(WorkflowStatus::Complete)],
).await?;
```

## API

| Type | Trait |
|------|-------|
| `OpenSearchIndex` | `SearchIndex` |

Key methods:

| Method | Description |
|--------|-------------|
| `new(url, index_name)` | Create a provider pointing at an OpenSearch instance |
| `start()` | Create the index with mappings if it does not exist |
| `index_workflow(instance)` | Index or update a workflow document |
| `search(terms, skip, take, filters)` | Bool query with filters, returns `Page<WorkflowSearchResult>` |
| `client()` | Access the underlying `OpenSearch` client |
| `index_name()` | Get the configured index name |

### Search filters

| Filter | OpenSearch mapping |
|--------|--------------------|
| `SearchFilter::Status(status)` | `term` query on `status` (keyword) |
| `SearchFilter::Reference(ref)` | `term` query on `reference` (keyword) |
| `SearchFilter::DateRange { field, before, after }` | `range` query on date fields |

Free-text terms run a `multi_match` across `description`, `reference`, and `workflow_definition_id`.

## Index mapping

The index uses the following field types:

| Field | Type |
|-------|------|
| `id` | keyword |
| `workflow_definition_id` | keyword |
| `version` | integer |
| `status` | keyword |
| `reference` | keyword |
| `description` | text |
| `data` | object (disabled -- stored but not indexed) |
| `create_time` | date |
| `complete_time` | date |

## Configuration

Constructor takes two arguments:

| Parameter | Example | Description |
|-----------|---------|-------------|
| `url` | `http://localhost:9200` | OpenSearch server URL |
| `index_name` | `wfe_workflows` | Index name for workflow documents |

Security plugin is not required. For local development, run OpenSearch with `DISABLE_SECURITY_PLUGIN=true`.

## Testing

Requires a running OpenSearch instance. Use the project docker-compose:

```sh
docker compose up -d opensearch
cargo test -p wfe-opensearch
```

Default test connection: `http://localhost:9200`

## License

MIT
