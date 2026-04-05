use std::sync::Arc;

use deno_core::op2;
use deno_core::OpState;

use crate::state::WfeState;

/// Create a WorkflowHost with in-memory providers and store it in state.
#[op2(fast)]
pub fn op_host_create(state: &mut OpState) -> Result<(), deno_error::JsErrorBox> {
    let wfe = state.borrow_mut::<WfeState>();
    if wfe.host.is_some() {
        return Err(deno_error::JsErrorBox::generic(
            "WorkflowHost already created",
        ));
    }

    let persistence = Arc::new(wfe_core::test_support::InMemoryPersistenceProvider::new());
    let lock = Arc::new(wfe_core::test_support::InMemoryLockProvider::new());
    let queue = Arc::new(wfe_core::test_support::InMemoryQueueProvider::new());
    let lifecycle = Arc::new(wfe_core::test_support::InMemoryLifecyclePublisher::new());

    let host = wfe::WorkflowHostBuilder::new()
        .use_persistence(persistence)
        .use_lock_provider(lock)
        .use_queue_provider(queue)
        .use_lifecycle(lifecycle)
        .build()
        .map_err(|e| deno_error::JsErrorBox::generic(format!("Failed to build host: {e}")))?;

    wfe.host = Some(Arc::new(host));
    Ok(())
}

/// Start the WorkflowHost background tasks.
#[op2]
pub async fn op_host_start(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
) -> Result<(), deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.start()
        .await
        .map_err(|e| deno_error::JsErrorBox::generic(format!("Failed to start host: {e}")))
}

/// Stop the WorkflowHost gracefully.
#[op2]
pub async fn op_host_stop(
    state: std::rc::Rc<std::cell::RefCell<OpState>>,
) -> Result<(), deno_error::JsErrorBox> {
    let host = {
        let s = state.borrow();
        let wfe = s.borrow::<WfeState>();
        wfe.host()?.clone()
    };
    host.stop().await;
    Ok(())
}
