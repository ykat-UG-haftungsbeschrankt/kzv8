/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use core::cell::UnsafeCell;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

const MAX_TASKS: usize = 16;

type TaskFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

struct Task {
    active: AtomicBool,
    ready: AtomicBool,
    future: UnsafeCell<Option<TaskFuture>>,
}

impl Task {
    const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            future: UnsafeCell::new(None),
        }
    }
}

struct Executor {
    tasks: [Task; MAX_TASKS],
}

impl Executor {
    const fn new() -> Self {
        Self {
            tasks: [const { Task::new() }; MAX_TASKS],
        }
    }
}

unsafe impl Sync for Executor {}

static EXECUTOR: Executor = Executor::new();
static POLLING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpawnError {
    Full,
}

pub(crate) fn spawn<F>(future: F) -> Result<(), SpawnError>
where
    F: Future<Output = ()> + 'static,
{
    let future: TaskFuture = Box::pin(future);
    for task in &EXECUTOR.tasks {
        if task
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            continue;
        }
        unsafe { *task.future.get() = Some(future) };
        task.ready.store(true, Ordering::Release);
        crate::arch::notify_executor();
        return Ok(());
    }
    Err(SpawnError::Full)
}

fn poll_ready() {
    if POLLING.swap(true, Ordering::Acquire) {
        return;
    }
    let _polling = PollingGuard;

    for task in &EXECUTOR.tasks {
        if !task.active.load(Ordering::Acquire)
            || !task.ready.swap(false, Ordering::AcqRel)
        {
            continue;
        }

        let waker = unsafe {
            Waker::from_raw(RawWaker::new(
                task as *const Task as *const (),
                &TASK_WAKER_VTABLE,
            ))
        };
        let mut context = Context::from_waker(&waker);
        let poll_result = {
            let future_slot = unsafe { &mut *task.future.get() };
            let Some(future) = future_slot.as_mut() else {
                task.active.store(false, Ordering::Release);
                continue;
            };
            future.as_mut().poll(&mut context)
        };

        if poll_result.is_ready() {
            unsafe { *task.future.get() = None };
            task.ready.store(false, Ordering::Relaxed);
            task.active.store(false, Ordering::Release);
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
        let interrupt_state = crate::arch::interrupts::disable();
        if has_ready_tasks() {
            crate::arch::interrupts::restore(interrupt_state);
            continue;
        }
        crate::arch::wait_for_interrupt();
    }
}

unsafe fn clone_task_waker(data: *const ()) -> RawWaker {
    RawWaker::new(data, &TASK_WAKER_VTABLE)
}

unsafe fn wake_task(data: *const ()) {
    let task = unsafe { &*data.cast::<Task>() };
    task.ready.store(true, Ordering::Release);
    crate::arch::notify_executor();
}

unsafe fn wake_task_by_ref(data: *const ()) {
    unsafe { wake_task(data) };
}

unsafe fn drop_task_waker(_data: *const ()) {}

static TASK_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
    clone_task_waker,
    wake_task,
    wake_task_by_ref,
    drop_task_waker,
);

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