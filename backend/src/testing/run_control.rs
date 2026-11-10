use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use tokio::sync::Notify;

#[derive(Default)]
struct GateState {
    entered: AtomicBool,
    released: AtomicBool,
    finished: AtomicBool,
    entered_notify: Notify,
    released_notify: Notify,
    finished_notify: Notify,
}

static GATES: LazyLock<Mutex<HashMap<String, Arc<GateState>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// e2e 并发测试的确定性控制闸门；仅在 `e2e-test` feature 下存在。
pub struct RunControlGate {
    key: String,
    state: Arc<GateState>,
}

impl RunControlGate {
    pub fn before_registration(data_dir: &Path, client_run_id: &str) -> Self {
        Self::arm(key(data_dir, "registration", client_run_id))
    }

    pub fn result_publication(data_dir: &Path, run_id: i32) -> Self {
        Self::arm(key(data_dir, "publication", &run_id.to_string()))
    }

    fn arm(key: String) -> Self {
        let state = Arc::new(GateState::default());
        GATES
            .lock()
            .expect("run control gates")
            .insert(key.clone(), state.clone());
        Self { key, state }
    }

    pub async fn wait_until_entered(&self) {
        while !self.state.entered.load(Ordering::Acquire) {
            self.state.entered_notify.notified().await;
        }
    }

    pub fn release(&self) {
        self.state.released.store(true, Ordering::Release);
        self.state.released_notify.notify_waiters();
    }

    pub async fn wait_until_finished(&self) {
        while !self.state.finished.load(Ordering::Acquire) {
            self.state.finished_notify.notified().await;
        }
    }
}

impl Drop for RunControlGate {
    fn drop(&mut self) {
        self.release();
        GATES.lock().expect("run control gates").remove(&self.key);
    }
}

pub(crate) async fn pause_before_registration(data_dir: &Path, client_run_id: &str) {
    pause(&key(data_dir, "registration", client_run_id)).await;
}

pub(crate) async fn pause_result_publication(data_dir: &Path, run_id: i32) {
    pause(&key(data_dir, "publication", &run_id.to_string())).await;
}

pub(crate) fn finish_result_publication(data_dir: &Path, run_id: i32) {
    let Some(state) = GATES
        .lock()
        .expect("run control gates")
        .get(&key(data_dir, "publication", &run_id.to_string()))
        .cloned()
    else {
        return;
    };
    state.finished.store(true, Ordering::Release);
    state.finished_notify.notify_waiters();
}

async fn pause(key: &str) {
    let Some(state) = GATES.lock().expect("run control gates").get(key).cloned() else {
        return;
    };
    state.entered.store(true, Ordering::Release);
    state.entered_notify.notify_waiters();
    while !state.released.load(Ordering::Acquire) {
        state.released_notify.notified().await;
    }
}

fn key(data_dir: &Path, kind: &str, id: &str) -> String {
    format!("{}:{kind}:{id}", data_dir.display())
}
