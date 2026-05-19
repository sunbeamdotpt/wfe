use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_core::traits::ArtifactStore;
use wfe_yaml::load_single_workflow_from_str;

#[tokio::test]
async fn git_repo_step_compiles() {
    let yaml = r#"
workflow:
  id: test-git-repo
  version: 1
  steps:
    - name: clone
      type: git-repo
      config:
        run: https://github.com/sunbeamdotpt/sunbeam
        branch: main
        working_dir: repo
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    assert_eq!(compiled.definition.id, "test-git-repo");
    assert_eq!(compiled.definition.steps.len(), 1);
}

#[tokio::test]
async fn git_repo_step_with_artifact_override_skips_clone() {
    // Create a temp artifact store and seed it with a dummy artifact.
    let tmp_dir = tempfile::tempdir().unwrap();
    let store = wfe_core::LocalArtifactStore::open(tmp_dir.path()).await.unwrap();

    // Create a dummy tar.gz artifact.
    let mut buf = Vec::new();
    {
        let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_path("README.md").unwrap();
        header.set_size(5);
        header.set_cksum();
        tar.append(&header, b"hello".as_slice()).unwrap();
        let enc = tar.into_inner().unwrap();
        enc.finish().unwrap();
    }

    let reader: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>> =
        Box::pin(std::io::Cursor::new(buf));
    let artifact = store.put(reader).await.unwrap();

    let yaml = r#"
workflow:
  id: test-git-repo-override
  version: 1
  steps:
    - name: clone
      type: git-repo
      config:
        run: https://example.com/repo.git
        input: repo_artifact
        working_dir: repo
"#;

    let data = serde_json::json!({
        "__wfe_working_dir": tmp_dir.path().to_str().unwrap(),
        "repo_artifact": { "__wfe_artifact": artifact.digest }
    });

    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .use_artifact_store(Arc::new(store))
        .build()
        .unwrap();

    for (key, factory) in compiled.step_factories {
        host.register_step_factory(&key, factory).await;
    }
    host.register_workflow_definition(compiled.definition.clone()).await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        &compiled.definition.id,
        compiled.definition.version,
        data,
        Duration::from_secs(30),
    )
    .await
    .unwrap();

    host.stop().await;

    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Verify the artifact was extracted to the working directory.
    let readme = tmp_dir.path().join("repo/README.md");
    assert!(readme.exists(), "artifact should have been extracted to {}", readme.display());
    let contents = tokio::fs::read_to_string(&readme).await.unwrap();
    assert_eq!(contents, "hello");
}
