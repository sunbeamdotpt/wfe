#![warn(missing_docs)]
//! wfe-server — standalone gRPC + HTTP server for the Workflow Engine.
//!
//! This binary wires together a [`WorkflowHost`](wfe::WorkflowHost) with
//! concrete provider implementations (SQLite, Postgres, Valkey, etc.), exposes
//! it over gRPC, and serves HTTP endpoints for webhooks, health checks, and
//! schema discovery.
//!
//! # Architecture
//! 1. Parse CLI arguments and load configuration.
//! 2. Initialize tracing.
//! 3. Build persistence, lock, and queue providers from config.
//! 4. Build lifecycle event broadcaster.
//! 5. Build optional log search index (OpenSearch).
//! 6. Build log store.
//! 7. Assemble the [`WorkflowHost`](wfe::WorkflowHost).
//! 8. Auto-load YAML workflow definitions from disk.
//! 9. Start the workflow engine.
//! 10. Start gRPC + HTTP servers with graceful shutdown.
//!
//! # Configuration
//! See `wfe-server --help` for CLI flags. Configuration can also be provided
//! via a TOML file.

mod auth;
mod config;
mod grpc;
mod lifecycle_bus;
mod log_search;
mod log_store;
mod webhook;

use std::sync::Arc;

use clap::Parser;
use tonic::transport::Server;
use tracing_subscriber::EnvFilter;
use wfe::WorkflowHostBuilder;
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_server_protos::wfe::v1::wfe_server::WfeServer;

use crate::config::{Cli, PersistenceConfig, QueueConfig};
use crate::grpc::WfeService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Parse CLI + load config.
    let cli = Cli::parse();
    let config = config::load(&cli);

    // 2. Init tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    tracing::info!(
        grpc_addr = %config.grpc_addr,
        http_addr = %config.http_addr,
        "starting wfe-server"
    );

    // 3. Build providers based on config.
    let (persistence, lock, queue): (
        Arc<dyn wfe_core::traits::PersistenceProvider>,
        Arc<dyn wfe_core::traits::DistributedLockProvider>,
        Arc<dyn wfe_core::traits::QueueProvider>,
    ) = match (&config.persistence, &config.queue) {
        (PersistenceConfig::Sqlite { path }, QueueConfig::InMemory) => {
            tracing::info!(path = %path, "using SQLite + in-memory queue");
            let persistence = Arc::new(
                wfe_sqlite::SqlitePersistenceProvider::new(path)
                    .await
                    .expect("failed to init SQLite"),
            );
            let lock = Arc::new(InMemoryLockProvider::new());
            let queue = Arc::new(InMemoryQueueProvider::new());
            (persistence, lock, queue)
        }
        (PersistenceConfig::Postgres { url }, QueueConfig::Valkey { url: valkey_url }) => {
            tracing::info!("using Postgres + Valkey");
            let persistence = Arc::new(
                wfe_postgres::PostgresPersistenceProvider::new(url)
                    .await
                    .expect("failed to init Postgres"),
            );
            let lock = Arc::new(
                wfe_valkey::ValkeyLockProvider::new(valkey_url, "wfe")
                    .await
                    .expect("failed to init Valkey lock"),
            );
            let queue = Arc::new(
                wfe_valkey::ValkeyQueueProvider::new(valkey_url, "wfe")
                    .await
                    .expect("failed to init Valkey queue"),
            );
            (
                persistence as Arc<dyn wfe_core::traits::PersistenceProvider>,
                lock as Arc<dyn wfe_core::traits::DistributedLockProvider>,
                queue as Arc<dyn wfe_core::traits::QueueProvider>,
            )
        }
        _ => {
            tracing::info!("using in-memory providers (dev mode)");
            let persistence = Arc::new(InMemoryPersistenceProvider::new());
            let lock = Arc::new(InMemoryLockProvider::new());
            let queue = Arc::new(InMemoryQueueProvider::new());
            (
                persistence as Arc<dyn wfe_core::traits::PersistenceProvider>,
                lock as Arc<dyn wfe_core::traits::DistributedLockProvider>,
                queue as Arc<dyn wfe_core::traits::QueueProvider>,
            )
        }
    };

    // 4. Build lifecycle broadcaster.
    let lifecycle_bus = Arc::new(lifecycle_bus::BroadcastLifecyclePublisher::new(4096));

    // 5. Build log search index (optional, needs to exist before log store).
    let log_search_index = if let Some(ref search_config) = config.search {
        match log_search::LogSearchIndex::new(&search_config.url) {
            Ok(index) => {
                let index = Arc::new(index);
                if let Err(e) = index.ensure_index().await {
                    tracing::warn!(error = %e, "failed to create log search index");
                }
                tracing::info!(url = %search_config.url, "log search enabled");
                Some(index)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to connect to OpenSearch");
                None
            }
        }
    } else {
        None
    };

    // 6. Build log store (with optional search indexing).
    let log_store = {
        let store = log_store::LogStore::new();
        if let Some(ref index) = log_search_index {
            Arc::new(store.with_search(index.clone()))
        } else {
            Arc::new(store)
        }
    };

    // 7. Build WorkflowHost with lifecycle + log_sink.
    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence)
        .use_lock_provider(lock)
        .use_queue_provider(queue)
        .use_lifecycle(lifecycle_bus.clone() as Arc<dyn wfe_core::traits::LifecyclePublisher>)
        .use_log_sink(log_store.clone() as Arc<dyn wfe_core::traits::LogSink>)
        .build()
        .expect("failed to build workflow host");

    // 8. Auto-load YAML definitions.
    if let Some(ref dir) = config.workflows_dir {
        load_yaml_definitions(&host, dir).await;
    }

    // 9. Start the workflow engine.
    host.start().await.expect("failed to start workflow host");
    tracing::info!("workflow engine started");

    let host = Arc::new(host);

    // 10. Build gRPC service.
    let mut wfe_service = WfeService::new(host.clone(), lifecycle_bus, log_store);
    if let Some(index) = log_search_index {
        wfe_service = wfe_service.with_log_search(index);
    }
    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter.set_serving::<WfeServer<WfeService>>().await;

    // 11. Build auth state.
    let auth_state = Arc::new(auth::AuthState::new(config.auth.clone()).await);
    let auth_interceptor = auth::make_interceptor(auth_state);

    // 12. Build axum HTTP server for webhooks.
    let webhook_state = webhook::WebhookState {
        host: host.clone(),
        config: config.clone(),
    };

    // HIGH-08: Limit webhook payload size to 2 MB to prevent OOM DoS.
    let http_router = axum::Router::new()
        .route(
            "/webhooks/events",
            axum::routing::post(webhook::handle_generic_event),
        )
        .route(
            "/webhooks/github",
            axum::routing::post(webhook::handle_github_webhook),
        )
        .route(
            "/webhooks/gitea",
            axum::routing::post(webhook::handle_gitea_webhook),
        )
        .route("/healthz", axum::routing::get(webhook::health_check))
        .route(
            "/schema/workflow.proto",
            axum::routing::get(serve_proto_schema),
        )
        .route(
            "/schema/workflow.json",
            axum::routing::get(serve_json_schema),
        )
        .route(
            "/schema/workflow.yaml",
            axum::routing::get(serve_yaml_example),
        )
        .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(webhook_state);

    // 12. Run gRPC + HTTP servers with graceful shutdown.
    let grpc_addr = config.grpc_addr;
    let http_addr = config.http_addr;
    tracing::info!(%grpc_addr, %http_addr, "servers listening");

    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(wfe_server_protos::FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("failed to build reflection service");

    let grpc_server = Server::builder()
        .add_service(health_service)
        .add_service(reflection_service)
        .add_service(WfeServer::with_interceptor(wfe_service, auth_interceptor))
        .serve(grpc_addr);

    let http_listener = tokio::net::TcpListener::bind(http_addr)
        .await
        .expect("failed to bind HTTP address");
    let http_server = axum::serve(http_listener, http_router);

    tokio::select! {
        result = grpc_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "gRPC server error");
            }
        }
        result = http_server => {
            if let Err(e) = result {
                tracing::error!(error = %e, "HTTP server error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutdown signal received");
        }
    }

    // 9. Graceful shutdown.
    host.stop().await;
    tracing::info!("wfe-server stopped");
    Ok(())
}

async fn load_yaml_definitions(host: &wfe::WorkflowHost, dir: &std::path::Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "failed to read workflows directory");
            return;
        }
    };

    let config = std::collections::HashMap::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "yaml" || ext == "yml")
        {
            match wfe_yaml::load_workflow_from_str(
                &std::fs::read_to_string(&path).unwrap_or_default(),
                &config,
            ) {
                Ok(workflows) => {
                    for compiled in workflows {
                        for (key, factory) in compiled.step_factories {
                            host.register_step_factory(&key, factory).await;
                        }
                        let id = compiled.definition.id.clone();
                        let version = compiled.definition.version;
                        host.register_workflow_definition(compiled.definition).await;
                        tracing::info!(id = %id, version, path = %path.display(), "loaded workflow definition");
                    }
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "failed to compile workflow");
                }
            }
        }
    }
}

/// Serve the raw .proto schema file.
async fn serve_proto_schema() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        include_str!("../proto/wfe/v1/wfe.proto"),
    )
}

/// Serve the auto-generated JSON Schema for workflow YAML definitions.
async fn serve_json_schema() -> impl axum::response::IntoResponse {
    let schema = wfe_yaml::schema::generate_json_schema();
    axum::Json(schema)
}

/// Serve the auto-generated JSON Schema as YAML.
async fn serve_yaml_example() -> impl axum::response::IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/yaml; charset=utf-8")],
        wfe_yaml::schema::generate_yaml_schema(),
    )
}
