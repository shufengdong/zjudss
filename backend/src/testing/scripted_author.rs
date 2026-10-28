use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::Notify;

use crate::agent::{ScriptAuthor, ScriptAuthorError, ScriptGenerationRequest, ScriptRepairRequest};

#[derive(Debug, Clone)]
pub enum AuthorTurn {
    Script(String),
    Error(String),
    Wait(String),
}

#[derive(Default)]
struct AuthorBarrier {
    entered: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    entered_notify: Notify,
    released_notify: Notify,
}

#[derive(Default)]
struct Inner {
    turns: Mutex<VecDeque<AuthorTurn>>,
    barriers: Mutex<HashMap<String, Arc<AuthorBarrier>>>,
    trace: Mutex<Vec<String>>,
    repair_requests: Mutex<Vec<ScriptRepairRequest>>,
}

#[derive(Clone, Default)]
pub struct ScriptedAuthor {
    inner: Arc<Inner>,
}

impl ScriptedAuthor {
    pub fn enqueue(&self, turn: AuthorTurn) {
        if let AuthorTurn::Wait(id) = &turn {
            self.inner
                .barriers
                .lock()
                .expect("author barrier registry")
                .entry(id.clone())
                .or_default();
        }
        self.inner
            .turns
            .lock()
            .expect("author turn queue")
            .push_back(turn);
    }

    pub fn release(&self, id: &str) -> bool {
        let barrier = self
            .inner
            .barriers
            .lock()
            .expect("author barrier registry")
            .get(id)
            .cloned();
        if let Some(barrier) = barrier {
            barrier
                .released
                .store(true, std::sync::atomic::Ordering::Release);
            barrier.released_notify.notify_waiters();
            true
        } else {
            false
        }
    }

    pub async fn wait_until_blocked(&self, id: &str) -> bool {
        let barrier = self
            .inner
            .barriers
            .lock()
            .expect("author barrier registry")
            .get(id)
            .cloned();
        let Some(barrier) = barrier else {
            return false;
        };
        while !barrier.entered.load(std::sync::atomic::Ordering::Acquire) {
            barrier.entered_notify.notified().await;
        }
        true
    }

    pub fn trace(&self) -> Vec<String> {
        self.inner.trace.lock().expect("author trace").clone()
    }

    pub fn repair_requests(&self) -> Vec<ScriptRepairRequest> {
        self.inner
            .repair_requests
            .lock()
            .expect("author repair requests")
            .clone()
    }

    async fn next(&self, operation: &str) -> Result<String, ScriptAuthorError> {
        self.inner
            .trace
            .lock()
            .expect("author trace")
            .push(operation.into());
        loop {
            let turn = self
                .inner
                .turns
                .lock()
                .expect("author turn queue")
                .pop_front()
                .ok_or_else(|| ScriptAuthorError::provider("没有可用的 scripted author 场景"))?;
            match turn {
                AuthorTurn::Script(script) => return Ok(script),
                AuthorTurn::Error(message) => return Err(ScriptAuthorError::provider(message)),
                AuthorTurn::Wait(id) => {
                    let barrier = self
                        .inner
                        .barriers
                        .lock()
                        .expect("author barrier registry")
                        .get(&id)
                        .cloned()
                        .expect("registered author barrier");
                    barrier
                        .entered
                        .store(true, std::sync::atomic::Ordering::Release);
                    barrier.entered_notify.notify_waiters();
                    while !barrier.released.load(std::sync::atomic::Ordering::Acquire) {
                        barrier.released_notify.notified().await;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl ScriptAuthor for ScriptedAuthor {
    async fn generate(
        &self,
        _request: ScriptGenerationRequest,
    ) -> Result<String, ScriptAuthorError> {
        self.next("generate").await
    }

    async fn repair(&self, request: ScriptRepairRequest) -> Result<String, ScriptAuthorError> {
        self.inner
            .repair_requests
            .lock()
            .expect("author repair requests")
            .push(request);
        self.next("repair").await
    }
}
