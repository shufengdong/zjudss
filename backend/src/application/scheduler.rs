use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::auth::UserId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppError {
    QueueFull,
}

pub type SchedulerError = AppError;

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull => write!(f, "Agent 请求队列已满，请稍后重试"),
        }
    }
}

impl std::error::Error for AppError {}

#[derive(Clone)]
pub struct RunScheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    global: Arc<Semaphore>,
    owner_limit: usize,
    queue_capacity: usize,
    waiting: AtomicUsize,
    active: AtomicUsize,
    owners: Mutex<HashMap<UserId, Weak<Semaphore>>>,
    sessions: Mutex<HashMap<(UserId, i32), Weak<Semaphore>>>,
}

impl RunScheduler {
    pub fn new(global_limit: usize, owner_limit: usize, queue_capacity: usize) -> Self {
        assert!(global_limit > 0);
        assert!(owner_limit > 0);
        assert!(queue_capacity > 0);
        Self {
            inner: Arc::new(SchedulerInner {
                global: Arc::new(Semaphore::new(global_limit)),
                owner_limit,
                queue_capacity,
                waiting: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                owners: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn try_admit(&self, scope: &UserId, session_id: i32) -> Result<RunTicket, AppError> {
        let owner = semaphore_for(&self.inner.owners, scope.clone(), self.inner.owner_limit);
        let session = semaphore_for(&self.inner.sessions, (scope.clone(), session_id), 1);

        // 已有等待者时，新请求也进入公平 semaphore 队列，避免持续到来的请求插队。
        if self.waiting_count() == 0
            && let Ok(owner_permit) = owner.clone().try_acquire_owned()
            && let Ok(session_permit) = session.clone().try_acquire_owned()
            && let Ok(global_permit) = self.inner.global.clone().try_acquire_owned()
        {
            self.inner.active.fetch_add(1, Ordering::SeqCst);
            return Ok(RunTicket {
                inner: self.inner.clone(),
                owner,
                session,
                admission: Some(Admission::Active(ActiveAdmission {
                    _owner: owner_permit,
                    _session: session_permit,
                    _global: global_permit,
                    _counter: ActiveCounter::new(self.inner.clone()),
                })),
            });
        }

        reserve_waiting(&self.inner)?;
        Ok(RunTicket {
            inner: self.inner.clone(),
            owner,
            session,
            admission: Some(Admission::Queued(WaitingCounter::new(self.inner.clone()))),
        })
    }

    pub fn active_count(&self) -> usize {
        self.inner.active.load(Ordering::SeqCst)
    }

    pub fn waiting_count(&self) -> usize {
        self.inner.waiting.load(Ordering::SeqCst)
    }
}

fn semaphore_for<K>(
    map: &Mutex<HashMap<K, Weak<Semaphore>>>,
    key: K,
    permits: usize,
) -> Arc<Semaphore>
where
    K: Eq + std::hash::Hash,
{
    let mut map = map.lock().expect("scheduler keyed semaphore lock poisoned");
    map.retain(|_, semaphore| semaphore.strong_count() > 0);
    if let Some(semaphore) = map.get(&key).and_then(Weak::upgrade) {
        return semaphore;
    }
    let semaphore = Arc::new(Semaphore::new(permits));
    map.insert(key, Arc::downgrade(&semaphore));
    semaphore
}

fn reserve_waiting(inner: &Arc<SchedulerInner>) -> Result<(), AppError> {
    let mut current = inner.waiting.load(Ordering::SeqCst);
    loop {
        if current >= inner.queue_capacity {
            return Err(AppError::QueueFull);
        }
        match inner.waiting.compare_exchange(
            current,
            current + 1,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return Ok(()),
            Err(actual) => current = actual,
        }
    }
}

enum Admission {
    Active(ActiveAdmission),
    Queued(WaitingCounter),
}

struct ActiveAdmission {
    _owner: OwnedSemaphorePermit,
    _session: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
    _counter: ActiveCounter,
}

struct WaitingCounter {
    inner: Arc<SchedulerInner>,
    counted: bool,
}

impl WaitingCounter {
    fn new(inner: Arc<SchedulerInner>) -> Self {
        Self {
            inner,
            counted: true,
        }
    }

    fn release(&mut self) {
        if self.counted {
            self.inner.waiting.fetch_sub(1, Ordering::SeqCst);
            self.counted = false;
        }
    }
}

impl Drop for WaitingCounter {
    fn drop(&mut self) {
        self.release();
    }
}

struct ActiveCounter {
    inner: Arc<SchedulerInner>,
}

impl ActiveCounter {
    fn new(inner: Arc<SchedulerInner>) -> Self {
        Self { inner }
    }
}

impl Drop for ActiveCounter {
    fn drop(&mut self) {
        self.inner.active.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct RunTicket {
    inner: Arc<SchedulerInner>,
    owner: Arc<Semaphore>,
    session: Arc<Semaphore>,
    admission: Option<Admission>,
}

impl fmt::Debug for RunTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RunTicket")
            .field("queued", &self.is_queued())
            .finish()
    }
}

impl RunTicket {
    pub fn is_queued(&self) -> bool {
        matches!(self.admission, Some(Admission::Queued(_)))
    }

    pub async fn run<F>(mut self, future: F) -> F::Output
    where
        F: Future,
    {
        let admission = self.admission.take().expect("run ticket can run only once");
        match admission {
            Admission::Active(active) => {
                let result = future.await;
                drop(active);
                result
            }
            Admission::Queued(mut waiting) => {
                // session → owner → global：同会话等待项不会占住 owner/global permit。
                let session = self
                    .session
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("scheduler session semaphore is never closed");
                let owner = self
                    .owner
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("scheduler owner semaphore is never closed");
                let global = self
                    .inner
                    .global
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("scheduler global semaphore is never closed");
                waiting.release();
                self.inner.active.fetch_add(1, Ordering::SeqCst);
                let active = ActiveAdmission {
                    _owner: owner,
                    _session: session,
                    _global: global,
                    _counter: ActiveCounter::new(self.inner.clone()),
                };
                let result = future.await;
                drop(active);
                result
            }
        }
    }
}
