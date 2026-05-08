use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use wfe::WorkflowHost;

use crate::bridge::StepRequest;
use crate::ops::builder::JsBuilderState;

/// Central state shared between all ops via `OpState`.
pub struct WfeState {
    /// Host.
    pub host: Option<Arc<WorkflowHost>>,
    /// Step request tx.
    pub step_request_tx: mpsc::Sender<StepRequest>,
    /// Step request rx.
    pub step_request_rx: Option<mpsc::Receiver<StepRequest>>,
    /// Builders.
    pub builders: HashMap<u32, JsBuilderState>,
    /// Next builder id.
    pub next_builder_id: u32,
    /// Inflight.
    pub inflight: HashMap<u32, oneshot::Sender<Result<serde_json::Value, String>>>,
    /// Next request id.
    pub next_request_id: u32,
}

impl WfeState {
    pub fn new(
        step_request_tx: mpsc::Sender<StepRequest>,
        step_request_rx: mpsc::Receiver<StepRequest>,
    ) -> Self {
        Self {
            host: None,
            step_request_tx,
            step_request_rx: Some(step_request_rx),
            builders: HashMap::new(),
            next_builder_id: 0,
            inflight: HashMap::new(),
            next_request_id: 0,
        }
    }

    pub fn host(&self) -> Result<&Arc<WorkflowHost>, deno_error::JsErrorBox> {
        self.host.as_ref().ok_or_else(|| {
            deno_error::JsErrorBox::generic(
                "WorkflowHost not created yet — call op_host_create first",
            )
        })
    }

    pub fn alloc_builder_id(&mut self) -> u32 {
        let id = self.next_builder_id;
        self.next_builder_id += 1;
        id
    }

    pub fn alloc_request_id(&mut self) -> u32 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_builder_id_increments() {
        let (tx, rx) = mpsc::channel(1);
        let mut state = WfeState::new(tx, rx);
        assert_eq!(state.alloc_builder_id(), 0);
        assert_eq!(state.alloc_builder_id(), 1);
        assert_eq!(state.alloc_builder_id(), 2);
    }

    #[test]
    fn alloc_request_id_increments() {
        let (tx, rx) = mpsc::channel(1);
        let mut state = WfeState::new(tx, rx);
        assert_eq!(state.alloc_request_id(), 0);
        assert_eq!(state.alloc_request_id(), 1);
    }

    #[test]
    fn host_returns_error_when_not_created() {
        let (tx, rx) = mpsc::channel(1);
        let state = WfeState::new(tx, rx);
        assert!(state.host().is_err());
    }
}
