use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    EventCommit,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FaultInjector;

static EVENT_COMMIT_FAILURES: AtomicUsize = AtomicUsize::new(0);

impl FaultInjector {
    pub fn fail_next(&self, point: FaultPoint) {
        match point {
            FaultPoint::EventCommit => {
                EVENT_COMMIT_FAILURES.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    pub(crate) fn take(&self, point: FaultPoint) -> bool {
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
