use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use crate::application::EventBus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    EventCommit,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FaultInjector {
    scope_id: Option<usize>,
}

static EVENT_COMMIT_FAILURES: AtomicUsize = AtomicUsize::new(0);
static EVENT_COMMIT_FAILURES_BY_SESSION: LazyLock<Mutex<HashMap<(usize, i32), usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl FaultInjector {
    pub fn for_event_bus(events: &EventBus) -> Self {
        Self {
            scope_id: Some(events.fault_scope_id()),
        }
    }

    pub fn fail_next(&self, point: FaultPoint) {
        match point {
            FaultPoint::EventCommit => {
                EVENT_COMMIT_FAILURES.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    pub fn fail_next_for_session(&self, point: FaultPoint, session_id: i32) {
        let scope_id = self.scope_id.expect("session fault requires app scope");
        let failures = match point {
            FaultPoint::EventCommit => &EVENT_COMMIT_FAILURES_BY_SESSION,
        };
        let mut failures = failures.lock().expect("session fault counter");
        *failures.entry((scope_id, session_id)).or_default() += 1;
    }

    pub(crate) fn take(&self, point: FaultPoint, session_id: i32) -> bool {
        if let Some(scope_id) = self.scope_id {
            let failures = match point {
                FaultPoint::EventCommit => &EVENT_COMMIT_FAILURES_BY_SESSION,
            };
            let mut failures = failures.lock().expect("session fault counter");
            let key = (scope_id, session_id);
            if let Some(remaining) = failures.get_mut(&key) {
                *remaining -= 1;
                if *remaining == 0 {
                    failures.remove(&key);
                }
                return true;
            }
        }

        let counter = match point {
            FaultPoint::EventCommit => &EVENT_COMMIT_FAILURES,
        };
        let mut current = counter.load(Ordering::SeqCst);
        while current > 0 {
            match counter.compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{FaultInjector, FaultPoint};

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn scoped_failure_is_not_consumed_by_another_session() {
        let _guard = TEST_LOCK.lock().unwrap();
        let injector = FaultInjector { scope_id: Some(1) };
        injector.fail_next_for_session(FaultPoint::EventCommit, 10);

        assert!(!injector.take(FaultPoint::EventCommit, 11));
        assert!(injector.take(FaultPoint::EventCommit, 10));
        assert!(!injector.take(FaultPoint::EventCommit, 10));
    }

    #[test]
    fn scoped_failure_is_not_consumed_by_another_app_with_the_same_session() {
        let _guard = TEST_LOCK.lock().unwrap();
        let first = FaultInjector { scope_id: Some(2) };
        let second = FaultInjector { scope_id: Some(3) };
        first.fail_next_for_session(FaultPoint::EventCommit, 10);

        assert!(!second.take(FaultPoint::EventCommit, 10));
        assert!(first.take(FaultPoint::EventCommit, 10));
    }

    #[test]
    fn global_failure_remains_available_to_any_session() {
        let _guard = TEST_LOCK.lock().unwrap();
        let injector = FaultInjector::default();
        injector.fail_next(FaultPoint::EventCommit);

        assert!(injector.take(FaultPoint::EventCommit, 42));
        assert!(!injector.take(FaultPoint::EventCommit, 42));
    }
}
