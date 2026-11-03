use core::sync::atomic::{AtomicBool, Ordering};

use crate::scheduler;

static FUTURE_SMOKE_DONE: AtomicBool = AtomicBool::new(false);

pub(crate) fn run() -> bool {
    let future_smoke_started = scheduler::task::spawn(futures_smoke()).is_ok();
    if future_smoke_started {
        scheduler::task::run_until_stalled();
    }
    future_smoke_started && FUTURE_SMOKE_DONE.load(Ordering::Acquire)
}

async fn futures_smoke() {
    scheduler::task::yield_now().await;
    FUTURE_SMOKE_DONE.store(true, Ordering::Release);
}