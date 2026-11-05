/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use alloc::sync::{Arc, Weak};
use alloc::task::Wake;
use core::future::Future;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering};
use core::task::{Context, Poll, Waker};

use crate::kernel::arch::traits::{CpuArchitecture as _, InterruptArchitecture as _};
use crate::kernel::error::Error;
use crate::kernel::lib::list::{LinkedList, ListNode};

use super::sync::IrqSpinLock;

type TaskFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum TaskState {
    Ready,
    Waiting,
    Running,
    Complete,
}

struct Task {
    state: AtomicU8,
    notified: AtomicBool,
    node: AtomicPtr<ListNode<Arc<Task>>>,
    future: IrqSpinLock<Option<TaskFuture>>,
    waker: IrqSpinLock<Option<Waker>>,
}

impl Task {
    fn new(future: TaskFuture) -> Self {
        Self {
            state: AtomicU8::new(TaskState::Ready as u8),
            notified: AtomicBool::new(false),
            node: AtomicPtr::new(core::ptr::null_mut()),
            future: IrqSpinLock::new(Some(future)),
            waker: IrqSpinLock::new(None),
        }
    }

    fn state(&self) -> TaskState {
        match self.state.load(Ordering::Acquire) {
            value if value == TaskState::Ready as u8 => TaskState::Ready,
            value if value == TaskState::Waiting as u8 => TaskState::Waiting,
            value if value == TaskState::Running as u8 => TaskState::Running,
            _ => TaskState::Complete,
        }
    }

    fn set_state(&self, state: TaskState) {
        self.state.store(state as u8, Ordering::Release);
    }
}

struct Executor {
    ready: LinkedList<Arc<Task>>,
    waiting: LinkedList<Arc<Task>>,
    running: LinkedList<Arc<Task>>,
}

impl Executor {
    const fn new() -> Self {
        Self {
            ready: LinkedList::new(),
            waiting: LinkedList::new(),
            running: LinkedList::new(),
        }
    }
}

static EXECUTOR: IrqSpinLock<Executor> = IrqSpinLock::new(Executor::new());
static POLLING: AtomicBool = AtomicBool::new(false);

struct TaskWaker {
    task: Weak<Task>,
}

impl TaskWaker {
    fn wake_task(&self) {
        let Some(task) = self.task.upgrade() else {
            return;
        };

        let notify = {
            let mut executor = EXECUTOR.lock();
            match task.state() {
                TaskState::Waiting => {
                    let Some(node) = NonNull::new(task.node.load(Ordering::Acquire)) else {
                        return;
                    };
                    // SAFETY: the task state and queue membership are protected by EXECUTOR.
                    let removed = unsafe { ListNode::remove(&mut executor.waiting, node) };
                    let Some(node) = removed else {
                        return;
                    };
                    task.set_state(TaskState::Ready);
                    executor.ready.push_back(node);
                    true
                }
                TaskState::Running => {
                    task.notified.store(true, Ordering::Release);
                    false
                }
                TaskState::Ready | TaskState::Complete => false,
            }
        };
        if notify {
            crate::kernel::arch::CpuArchitecture::notify_executor();
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
    let task = Arc::new(Task::new(Box::pin(future)));
    *task.waker.lock() = Some(Waker::from(Arc::new(TaskWaker {
        task: Arc::downgrade(&task),
    })));
    let mut node = Box::new(ListNode::new(Arc::clone(&task)));
    let node_pointer = NonNull::from(node.as_mut());
    task.node.store(node_pointer.as_ptr(), Ordering::Release);
    EXECUTOR.lock().ready.push_back(node);
    crate::kernel::arch::CpuArchitecture::notify_executor();
    Ok(())
}

fn poll_ready() {
    if POLLING.swap(true, Ordering::Acquire) {
        return;
    }
    let _polling = PollingGuard;

    loop {
        let task = {
            let mut executor = EXECUTOR.lock();
            let Some(node) = executor.ready.pop_front() else {
                return;
            };
            let task = Arc::clone(&**node);
            task.set_state(TaskState::Running);
            task.notified.store(false, Ordering::Relaxed);
            executor.running.push_back(node);
            task
        };

        let Some(mut future) = task.future.lock().take() else {
            finish_task(&task);
            continue;
        };
        let Some(waker) = task.waker.lock().as_ref().cloned() else {
            finish_task(&task);
            continue;
        };
        let mut context = Context::from_waker(&waker);
        let poll_result = future.as_mut().poll(&mut context);

        if poll_result.is_ready() {
            finish_task(&task);
        } else {
            *task.future.lock() = Some(future);
            let notify = {
                let mut executor = EXECUTOR.lock();
                let node = NonNull::new(task.node.load(Ordering::Acquire))
                    .expect("running task has no list node");
                // SAFETY: the task remains in the running queue until removed here.
                let node = unsafe { ListNode::remove(&mut executor.running, node) }
                    .unwrap_or_else(|| unreachable!("running task is not queued"));
                if task.notified.swap(false, Ordering::AcqRel) {
                    task.set_state(TaskState::Ready);
                    executor.ready.push_back(node);
                    true
                } else {
                    task.set_state(TaskState::Waiting);
                    executor.waiting.push_back(node);
                    false
                }
            };
            if notify {
                crate::kernel::arch::CpuArchitecture::notify_executor();
            }
        }
    }
}

fn finish_task(task: &Task) {
    let mut executor = EXECUTOR.lock();
    let node = NonNull::new(task.node.load(Ordering::Acquire))
        .expect("running task has no list node");
    // SAFETY: the task remains in the running queue until removed here.
    let node = unsafe { ListNode::remove(&mut executor.running, node) }
        .unwrap_or_else(|| unreachable!("running task is not queued"));
    task.node.store(core::ptr::null_mut(), Ordering::Release);
    task.notified.store(false, Ordering::Relaxed);
    task.set_state(TaskState::Complete);
    drop(node);
}

struct PollingGuard;

impl Drop for PollingGuard {
    fn drop(&mut self) {
        POLLING.store(false, Ordering::Release);
    }
}

fn has_ready_tasks() -> bool {
    !EXECUTOR.lock().ready.is_empty()
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
        let interrupt_state = crate::kernel::arch::InterruptArchitecture::disable_interrupts();
        if has_ready_tasks() {
            crate::kernel::arch::InterruptArchitecture::restore_interrupts(interrupt_state);
            continue;
        }
        crate::kernel::arch::CpuArchitecture::wait_for_interrupt();
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