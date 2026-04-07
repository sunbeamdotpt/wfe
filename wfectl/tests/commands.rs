//! Drive the wfectl command handlers against a stub gRPC server. This tests
//! the full command -> client -> server -> response -> formatter pipeline.

mod stub;

use std::path::PathBuf;

use wfectl::client::build as build_client;
use wfectl::commands;
use wfectl::output::OutputFormat;

use stub::spawn_stub;

#[tokio::test]
async fn register_command_table_output() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();

    // Write a temp YAML file
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "workflow:\n  id: x\n  version: 1\n  steps: []").unwrap();

    let args = commands::register::RegisterArgs {
        file: tmp.path().to_path_buf(),
        config: vec![("key".into(), "val".into())],
    };
    commands::register::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn register_command_json_output() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "workflow:\n  id: x\n  version: 1\n  steps: []").unwrap();
    let args = commands::register::RegisterArgs {
        file: tmp.path().to_path_buf(),
        config: vec![],
    };
    commands::register::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn register_command_missing_file_errors() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::register::RegisterArgs {
        file: PathBuf::from("/nonexistent/file.yaml"),
        config: vec![],
    };
    let result = commands::register::run(args, client, OutputFormat::Table).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn definitions_list_table() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::definitions::DefinitionsArgs {
        cmd: commands::definitions::DefinitionsCmd::List,
    };
    commands::definitions::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn definitions_list_json() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::definitions::DefinitionsArgs {
        cmd: commands::definitions::DefinitionsCmd::List,
    };
    commands::definitions::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_command_with_inline_data() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::run::RunArgs {
        definition_id: "ci".into(),
        version: 1,
        data: None,
        data_json: Some(r#"{"key":"value"}"#.into()),
        name: None,
    };
    commands::run::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_command_with_data_file() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), r#"{"deploy":true}"#).unwrap();
    let args = commands::run::RunArgs {
        definition_id: "ci".into(),
        version: 2,
        data: Some(tmp.path().to_path_buf()),
        data_json: None,
        name: None,
    };
    commands::run::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_command_no_data() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::run::RunArgs {
        definition_id: "ci".into(),
        version: 1,
        data: None,
        data_json: None,
        name: None,
    };
    commands::run::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_command_invalid_json_errors() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::run::RunArgs {
        definition_id: "ci".into(),
        version: 1,
        data: None,
        data_json: Some("not json".into()),
        name: None,
    };
    let result = commands::run::run(args, client, OutputFormat::Table).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn get_command_table() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::get::GetArgs {
        workflow_id: "wf-1".into(),
    };
    commands::get::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn get_command_json() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::get::GetArgs {
        workflow_id: "wf-1".into(),
    };
    commands::get::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_command_no_filters() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::list::ListArgs {
        query: None,
        status: None,
        limit: 10,
        skip: 0,
    };
    commands::list::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_command_all_filters() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::list::ListArgs {
        query: Some("foo".into()),
        status: Some(commands::list::StatusFilter::Complete),
        limit: 50,
        skip: 10,
    };
    commands::list::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn list_command_each_status_variant() {
    use commands::list::StatusFilter;
    for status in [
        StatusFilter::Runnable,
        StatusFilter::Suspended,
        StatusFilter::Complete,
        StatusFilter::Terminated,
    ] {
        let (server, _seen) = spawn_stub().await;
        let client = build_client(&server, "test-token").await.unwrap();
        let args = commands::list::ListArgs {
            query: None,
            status: Some(status),
            limit: 10,
            skip: 0,
        };
        commands::list::run(args, client, OutputFormat::Table)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn cancel_suspend_resume_commands() {
    let (server, _seen) = spawn_stub().await;
    let client1 = build_client(&server, "test-token").await.unwrap();
    let client2 = build_client(&server, "test-token").await.unwrap();
    let client3 = build_client(&server, "test-token").await.unwrap();

    commands::cancel::run(
        commands::cancel::CancelArgs {
            workflow_id: "wf-1".into(),
        },
        client1,
    )
    .await
    .unwrap();

    commands::suspend::run(
        commands::suspend::SuspendArgs {
            workflow_id: "wf-1".into(),
        },
        client2,
    )
    .await
    .unwrap();

    commands::resume::run(
        commands::resume::ResumeArgs {
            workflow_id: "wf-1".into(),
        },
        client3,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn publish_command_with_inline_data() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::publish::PublishArgs {
        event_name: "order.paid".into(),
        event_key: "order-42".into(),
        data: None,
        data_json: Some(r#"{"amount":100}"#.into()),
    };
    commands::publish::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn publish_command_with_file() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), r#"{"amount":100}"#).unwrap();
    let args = commands::publish::PublishArgs {
        event_name: "order.paid".into(),
        event_key: "order-42".into(),
        data: Some(tmp.path().to_path_buf()),
        data_json: None,
    };
    commands::publish::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn publish_command_no_data() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::publish::PublishArgs {
        event_name: "test".into(),
        event_key: "x".into(),
        data: None,
        data_json: None,
    };
    commands::publish::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn watch_command_streams_events() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::watch::WatchArgs {
        workflow_id: Some("wf-1".into()),
    };
    commands::watch::run(args, client).await.unwrap();
}

#[tokio::test]
async fn watch_command_no_filter() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::watch::WatchArgs { workflow_id: None };
    commands::watch::run(args, client).await.unwrap();
}

#[tokio::test]
async fn logs_command_basic() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::logs::LogsArgs {
        workflow_id: "wf-1".into(),
        step: None,
        follow: false,
    };
    commands::logs::run(args, client).await.unwrap();
}

#[tokio::test]
async fn logs_command_with_step_filter() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::logs::LogsArgs {
        workflow_id: "wf-1".into(),
        step: Some("build".into()),
        follow: true,
    };
    commands::logs::run(args, client).await.unwrap();
}

#[tokio::test]
async fn search_logs_command_table() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::search_logs::SearchLogsArgs {
        query: "needle".into(),
        workflow: None,
        step: None,
        stream: None,
        limit: 10,
        skip: 0,
    };
    commands::search_logs::run(args, client, OutputFormat::Table)
        .await
        .unwrap();
}

#[tokio::test]
async fn search_logs_command_with_filters_json() {
    let (server, _seen) = spawn_stub().await;
    let client = build_client(&server, "test-token").await.unwrap();
    let args = commands::search_logs::SearchLogsArgs {
        query: "needle".into(),
        workflow: Some("wf-1".into()),
        step: Some("build".into()),
        stream: Some(commands::search_logs::StreamFilter::Stdout),
        limit: 100,
        skip: 5,
    };
    commands::search_logs::run(args, client, OutputFormat::Json)
        .await
        .unwrap();
}

#[tokio::test]
async fn search_logs_each_stream_variant() {
    use commands::search_logs::StreamFilter;
    for stream in [StreamFilter::Stdout, StreamFilter::Stderr] {
        let (server, _seen) = spawn_stub().await;
        let client = build_client(&server, "test-token").await.unwrap();
        let args = commands::search_logs::SearchLogsArgs {
            query: "x".into(),
            workflow: None,
            step: None,
            stream: Some(stream),
            limit: 10,
            skip: 0,
        };
        commands::search_logs::run(args, client, OutputFormat::Table)
            .await
            .unwrap();
    }
}
