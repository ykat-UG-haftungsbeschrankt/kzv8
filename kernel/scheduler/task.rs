/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};

use crate::kernel::error::Error;

use super::sync::IrqSpinLock;

type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

struct Task {
    active: AtomicBool,
    ready: AtomicBool,
    wakeable: AtomicBool,
    generation: AtomicUsize,
    future: IrqSpinLock<Option<TaskFuture>>,
    waker: IrqSpinLock<Option<Waker>>,
}

impl Task {
    const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            wakeable: AtomicBool::new(false),
            generation: AtomicUsize::new(0),
            future: IrqSpinLock::new(None),
            waker: IrqSpinLock::new(None),
        }
    }
}

struct Executor {
    tasks: [Task; Self::MAX_TASKS],
}

impl Executor {
    const MAX_TASKS: usize = 16;
    const fn new() -> Self {
        Self {
            tasks: [const { Task::new() }; Self::MAX_TASKS],
        }
    }
}

static EXECUTOR: Executor = Executor::new();
static POLLING: AtomicBool = AtomicBool::new(false);

struct TaskWaker {
    slot: usize,
    generation: usize,
}

impl TaskWaker {
    fn wake_task(&self) {
        if let Some(task) = EXECUTOR.tasks.get(self.slot) {
            let notify = {
                let _waker = task.waker.lock();
                if task.active.load(Ordering::Acquire)
                    && task.wakeable.load(Ordering::Acquire)
                    && task.generation.load(Ordering::Acquire) == self.generation
                {
                    task.ready.store(true, Ordering::Release);
                    true
                } else {
                    false
                }
            };
            if notify {
                <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::CpuArchitecture>::
                    notify_executor();
            }
        }
    }
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_task();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wake_task();
    }
}

pub(crate) fn spawn<F>(future: F) -> Result<(), Error>
where
    F: Future<Output = ()> + Send + 'static,
{
    let future: TaskFuture = Box::pin(future);
    for (slot, task) in EXECUTOR.tasks.iter().enumerate() {
        if task
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            continue;
        }
        task.ready.store(false, Ordering::Relaxed);
        task.wakeable.store(false, Ordering::Release);
        let generation = task
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        *task.future.lock() = Some(future);
        {
            let mut waker_slot = task.waker.lock();
            *waker_slot = Some(Waker::from(Arc::new(TaskWaker { slot, generation })));
            task.wakeable.store(true, Ordering::Release);
        }
        task.ready.store(true, Ordering::Release);
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::CpuArchitecture>::
            notify_executor();
        return Ok(());
    }
    Err(Error::Again)
}

fn poll_ready() {
    if POLLING.swap(true, Ordering::Acquire) {
        return;
    }
    let _polling = PollingGuard;

    for task in &EXECUTOR.tasks {
        if !task.active.load(Ordering::Acquire) || !task.ready.swap(false, Ordering::AcqRel) {
            continue;
        }

        let future = {
            let mut future_slot = task.future.lock();
            let Some(future) = future_slot.take() else {
                task.active.store(false, Ordering::Release);
                continue;
            };
            future
        };
        let Some(waker) = task.waker.lock().as_ref().cloned() else {
            *task.future.lock() = Some(future);
            task.ready.store(true, Ordering::Release);
            continue;
        };
        let mut context = Context::from_waker(&waker);
        let mut future = future;
        let poll_result = future.as_mut().poll(&mut context);

        if poll_result.is_ready() {
            {
                let mut waker_slot = task.waker.lock();
                task.wakeable.store(false, Ordering::Release);
                waker_slot.take();
            }
            task.ready.store(false, Ordering::Relaxed);
            task.active.store(false, Ordering::Release);
        } else {
            *task.future.lock() = Some(future);
        }
    }
}

struct PollingGuard;

impl Drop for PollingGuard {
    fn drop(&mut self) {
        POLLING.store(false, Ordering::Release);
    }
}

fn has_ready_tasks() -> bool {
    EXECUTOR
        .tasks
        .iter()
        .any(|task| task.active.load(Ordering::Acquire) && task.ready.load(Ordering::Acquire))
}

pub(crate) fn run_until_stalled() {
    if POLLING.load(Ordering::Acquire) {
        return;
    }
    loop {
        poll_ready();
        if !has_ready_tasks() {
            return;
        }
    }
}

pub(crate) fn run_forever() -> ! {
    loop {
        run_until_stalled();
        let interrupt_state = <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::
            InterruptArchitecture>::disable_interrupts();
        if has_ready_tasks() {
            <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::InterruptArchitecture>::
                restore_interrupts(interrupt_state);
            continue;
        }
        <crate::kernel::arch::MemoryArchitecture as crate::kernel::arch::CpuArchitecture>::
            wait_for_interrupt();
    }
}

pub(crate) struct YieldNow {
    yielded: bool,
}

pub(crate) fn yield_now() -> YieldNow {
    YieldNow { yielded: false }
}

impl Future for YieldNow {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.yielded {
            Poll::Ready(())
        } else {
            this.yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
