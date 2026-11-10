use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    EventCommit,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FaultInjector;

static EVENT_COMMIT_FAILURES: AtomicUsize = AtomicUsize::new(0);
static EVENT_COMMIT_FAILURES_BY_SESSION: LazyLock<Mutex<HashMap<i32, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl FaultInjector {
    pub fn fail_next(&self, point: FaultPoint) {
        match point {
            FaultPoint::EventCommit => {
                EVENT_COMMIT_FAILURES.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    pub fn fail_next_for_session(&self, point: FaultPoint, session_id: i32) {
        let failures = match point {
            FaultPoint::EventCommit => &EVENT_COMMIT_FAILURES_BY_SESSION,
        };
        let mut failures = failures.lock().expect("session fault counter");
        *failures.entry(session_id).or_default() += 1;
    }

    pub(crate) fn take(&self, point: FaultPoint, session_id: i32) -> bool {
        let failures = match point {
            FaultPoint::EventCommit => &EVENT_COMMIT_FAILURES_BY_SESSION,
        };
        let mut failures = failures.lock().expect("session fault counter");
        if let Some(remaining) = failures.get_mut(&session_id) {
            *remaining -= 1;
            if *remaining == 0 {
                failures.remove(&session_id);
            }
            return true;
        }
        drop(failures);

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
        FaultInjector.fail_next_for_session(FaultPoint::EventCommit, 10);

        assert!(!FaultInjector.take(FaultPoint::EventCommit, 11));
        assert!(FaultInjector.take(FaultPoint::EventCommit, 10));
        assert!(!FaultInjector.take(FaultPoint::EventCommit, 10));
    }

    #[test]
    fn global_failure_remains_available_to_any_session() {
        let _guard = TEST_LOCK.lock().unwrap();
        FaultInjector.fail_next(FaultPoint::EventCommit);

        assert!(FaultInjector.take(FaultPoint::EventCommit, 42));
        assert!(!FaultInjector.take(FaultPoint::EventCommit, 42));
    }
}
