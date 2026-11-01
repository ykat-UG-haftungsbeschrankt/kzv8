use core::ffi::c_void;
use core::ptr::addr_of_mut;
use core::slice::from_raw_parts_mut;
use core::sync::atomic::{AtomicUsize, Ordering};

const MAX_THREADS: usize = 8;
const STACK_SIZE: usize = 16 * 1024;
const MAIN_THREAD_ID: usize = 1;

const EAGAIN: i32 = 11;
const EBUSY: i32 = 16;
const EDEADLK: i32 = 35;
const EINVAL: i32 = 22;
const EPERM: i32 = 1;
const ESRCH: i32 = 3;

const JOINABLE: i32 = 0;
const DETACHED: i32 = 1;

type StartRoutine = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

#[repr(C)]
struct ThreadAttributes {
    stack_size: usize,
    detach_state: i32,
}

#[repr(C)]
struct PthreadMutex {
    owner: AtomicUsize,
}

#[repr(C)]
struct PthreadCondition {
    generation: AtomicUsize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThreadState {
    Unused,
    Running,
    Ready,
    Blocked,
    Exited,
}

#[derive(Clone, Copy)]
struct Thread {
    id: usize,
    stack_pointer: usize,
    state: ThreadState,
    start_routine: Option<StartRoutine>,
    argument: *mut c_void,
    result: *mut c_void,
    joiner: usize,
    detached: bool,
    errno: i32,
}

impl Thread {
    const EMPTY: Self = Self {
        id: 0,
        stack_pointer: 0,
        state: ThreadState::Unused,
        start_routine: None,
        argument: core::ptr::null_mut(),
        result: core::ptr::null_mut(),
        joiner: 0,
        detached: false,
        errno: 0,
    };
}

#[derive(Clone, Copy)]
#[repr(align(16))]
struct ThreadStack([u8; STACK_SIZE]);

static mut THREADS: [Thread; MAX_THREADS] = [Thread::EMPTY; MAX_THREADS];
static mut STACKS: [ThreadStack; MAX_THREADS] = [ThreadStack([0; STACK_SIZE]); MAX_THREADS];
static mut CURRENT_SLOT: usize = 0;
static mut NEXT_THREAD_ID: usize = MAIN_THREAD_ID + 1;
static mut INITIALIZED: bool = false;
static mut FALLBACK_ERRNO: i32 = 0;

pub(crate) fn initialize() {
    let interrupt_state = crate::arch::interrupts::disable();
    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        for index in 0..MAX_THREADS {
            threads.add(index).write(Thread::EMPTY);
        }
        threads.add(0).write(Thread {
            id: MAIN_THREAD_ID,
            state: ThreadState::Running,
            ..Thread::EMPTY
        });
        CURRENT_SLOT = 0;
        NEXT_THREAD_ID = MAIN_THREAD_ID + 1;
        INITIALIZED = true;
    }
    crate::arch::interrupts::restore(interrupt_state);
}

#[no_mangle]
pub extern "C" fn __errno_location() -> *mut i32 {
    unsafe {
        if INITIALIZED {
            let current = CURRENT_SLOT;
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            addr_of_mut!((*threads.add(current)).errno)
        } else {
            addr_of_mut!(FALLBACK_ERRNO)
        }
    }
}

unsafe fn find_thread(id: usize) -> Option<usize> {
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    for index in 0..MAX_THREADS {
        let thread = threads.add(index);
        if (*thread).state != ThreadState::Unused && (*thread).id == id {
            return Some(index);
        }
    }
    None
}

unsafe fn find_ready(current: usize) -> Option<usize> {
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    for distance in 1..=MAX_THREADS {
        let index = (current + distance) % MAX_THREADS;
        if (*threads.add(index)).state == ThreadState::Ready {
            return Some(index);
        }
    }
    None
}

unsafe fn stack_slice(index: usize) -> &'static mut [u8] {
    let stacks = addr_of_mut!(STACKS).cast::<ThreadStack>();
    let stack = stacks.add(index);
    unsafe { from_raw_parts_mut(addr_of_mut!((*stack).0).cast::<u8>(), STACK_SIZE) }
}

fn switch_current(next_state: ThreadState) -> bool {
    let interrupt_state = crate::arch::interrupts::disable();
    let current = unsafe { CURRENT_SLOT };
    let next = unsafe { find_ready(current) };
    let Some(next) = next else {
        crate::arch::interrupts::restore(interrupt_state);
        return false;
    };

    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        (*threads.add(current)).state = next_state;
        (*threads.add(next)).state = ThreadState::Running;
        CURRENT_SLOT = next;
        let old_stack = addr_of_mut!((*threads.add(current)).stack_pointer);
        let new_stack = (*threads.add(next)).stack_pointer;
        crate::arch::switch_thread(&mut *old_stack, new_stack);
    }
    crate::arch::interrupts::restore(interrupt_state);
    true
}

fn current_thread_id() -> usize {
    let interrupt_state = crate::arch::interrupts::disable();
    let id = unsafe {
        if INITIALIZED {
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            (*threads.add(CURRENT_SLOT)).id
        } else {
            0
        }
    };
    crate::arch::interrupts::restore(interrupt_state);
    id
}

fn exit_current(result: *mut c_void) -> ! {
    let _interrupt_state = crate::arch::interrupts::disable();
    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let current = CURRENT_SLOT;
        (*threads.add(current)).result = result;
        (*threads.add(current)).state = ThreadState::Exited;

        let joiner = (*threads.add(current)).joiner;
        (*threads.add(current)).joiner = 0;
        if joiner != 0 {
            if let Some(joiner_slot) = find_thread(joiner) {
                if (*threads.add(joiner_slot)).state == ThreadState::Blocked {
                    (*threads.add(joiner_slot)).state = ThreadState::Ready;
                }
            }
        }

        if let Some(next) = find_ready(current) {
            (*threads.add(next)).state = ThreadState::Running;
            CURRENT_SLOT = next;
            let old_stack = addr_of_mut!((*threads.add(current)).stack_pointer);
            let new_stack = (*threads.add(next)).stack_pointer;
            crate::arch::switch_thread(&mut *old_stack, new_stack);
        }
    }
    crate::arch::halt()
}

extern "C" fn thread_trampoline() -> ! {
    let interrupt_state = crate::arch::interrupts::disable();
    let current = unsafe { CURRENT_SLOT };
    let (start_routine, argument) = unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let thread = threads.add(current);
        ((*thread).start_routine, (*thread).argument)
    };
    crate::arch::interrupts::restore(interrupt_state);

    if let Some(start_routine) = start_routine {
        let result = unsafe { start_routine(argument) };
        exit_current(result);
    }
    exit_current(core::ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn pthread_self() -> usize {
    current_thread_id()
}

#[no_mangle]
pub extern "C" fn pthread_equal(first: usize, second: usize) -> i32 {
    (first == second) as i32
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_init(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size: 0,
                detach_state: JOINABLE,
            });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_destroy(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size: 0,
                detach_state: JOINABLE,
            });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setdetachstate(
    attributes: *mut c_void,
    detach_state: i32,
) -> i32 {
    if attributes.is_null() || !matches!(detach_state, JOINABLE | DETACHED) {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).detach_state = detach_state };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getdetachstate(
    attributes: *const c_void,
    detach_state: *mut i32,
) -> i32 {
    if attributes.is_null() || detach_state.is_null() {
        return EINVAL;
    }
    unsafe { detach_state.write((*attributes.cast::<ThreadAttributes>()).detach_state) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_setstacksize(
    attributes: *mut c_void,
    stack_size: usize,
) -> i32 {
    if attributes.is_null() || stack_size != STACK_SIZE {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).stack_size = stack_size };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_attr_getstacksize(
    attributes: *const c_void,
    stack_size: *mut usize,
) -> i32 {
    if attributes.is_null() || stack_size.is_null() {
        return EINVAL;
    }
    let stored_size = unsafe { (*attributes.cast::<ThreadAttributes>()).stack_size };
    unsafe {
        stack_size.write(if stored_size == 0 {
            STACK_SIZE
        } else {
            stored_size
        })
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_create(
    thread_id: *mut usize,
    attributes: *const c_void,
    start_routine: Option<StartRoutine>,
    argument: *mut c_void,
) -> i32 {
    if thread_id.is_null() || start_routine.is_none() {
        return EINVAL;
    }

    let (stack_size, detach_state) = if attributes.is_null() {
        (0, JOINABLE)
    } else {
        let attributes = unsafe { &*attributes.cast::<ThreadAttributes>() };
        (attributes.stack_size, attributes.detach_state)
    };
    if !matches!(stack_size, 0 | STACK_SIZE) || !matches!(detach_state, JOINABLE | DETACHED) {
        return EINVAL;
    }

    let interrupt_state = crate::arch::interrupts::disable();
    let id = unsafe {
        if !INITIALIZED || NEXT_THREAD_ID == usize::MAX {
            crate::arch::interrupts::restore(interrupt_state);
            return EAGAIN;
        }
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let mut available = None;
        for index in 1..MAX_THREADS {
            let thread = threads.add(index);
            if (*thread).state == ThreadState::Unused
                || ((*thread).state == ThreadState::Exited && (*thread).detached)
            {
                available = Some(index);
                break;
            }
        }
        let Some(slot) = available else {
            crate::arch::interrupts::restore(interrupt_state);
            return EAGAIN;
        };

        let id = NEXT_THREAD_ID;
        NEXT_THREAD_ID += 1;
        let stack_pointer =
            crate::arch::initialize_thread_stack(stack_slice(slot), thread_trampoline);
        threads.add(slot).write(Thread {
            id,
            stack_pointer,
            state: ThreadState::Ready,
            start_routine,
            argument,
            detached: detach_state == DETACHED,
            ..Thread::EMPTY
        });
        id
    };
    unsafe { thread_id.write(id) };
    crate::arch::interrupts::restore(interrupt_state);
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_join(thread_id: usize, result: *mut *mut c_void) -> i32 {
    let interrupt_state = crate::arch::interrupts::disable();
    let (current_id, target_slot, target_state) = unsafe {
        if !INITIALIZED {
            crate::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        }
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let current_id = (*threads.add(CURRENT_SLOT)).id;
        if thread_id == current_id {
            crate::arch::interrupts::restore(interrupt_state);
            return EDEADLK;
        }
        let Some(target_slot) = find_thread(thread_id) else {
            crate::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        };
        let target = threads.add(target_slot);
        if (*target).detached || (*target).joiner != 0 {
            crate::arch::interrupts::restore(interrupt_state);
            return EINVAL;
        }
        (current_id, target_slot, (*target).state)
    };

    if target_state == ThreadState::Exited {
        let value = unsafe {
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            let target = threads.add(target_slot);
            let value = (*target).result;
            target.write(Thread::EMPTY);
            value
        };
        crate::arch::interrupts::restore(interrupt_state);
        if !result.is_null() {
            unsafe { result.write(value) };
        }
        return 0;
    }

    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        (*threads.add(target_slot)).joiner = current_id;
    }
    crate::arch::interrupts::restore(interrupt_state);

    if !switch_current(ThreadState::Blocked) {
        let interrupt_state = crate::arch::interrupts::disable();
        unsafe {
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            if let Some(target_slot) = find_thread(thread_id) {
                (*threads.add(target_slot)).joiner = 0;
            }
        }
        crate::arch::interrupts::restore(interrupt_state);
        return EDEADLK;
    }

    let interrupt_state = crate::arch::interrupts::disable();
    let value = unsafe {
        let Some(target_slot) = find_thread(thread_id) else {
            crate::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        };
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let target = threads.add(target_slot);
        if (*target).state != ThreadState::Exited {
            crate::arch::interrupts::restore(interrupt_state);
            return EDEADLK;
        }
        let value = (*target).result;
        target.write(Thread::EMPTY);
        value
    };
    crate::arch::interrupts::restore(interrupt_state);
    if !result.is_null() {
        unsafe { result.write(value) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_detach(thread_id: usize) -> i32 {
    let interrupt_state = crate::arch::interrupts::disable();
    let result = unsafe {
        let Some(slot) = find_thread(thread_id) else {
            crate::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        };
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let thread = threads.add(slot);
        if (*thread).detached || (*thread).joiner != 0 {
            crate::arch::interrupts::restore(interrupt_state);
            return EINVAL;
        }
        if (*thread).state == ThreadState::Exited {
            thread.write(Thread::EMPTY);
        } else {
            (*thread).detached = true;
        }
        0
    };
    crate::arch::interrupts::restore(interrupt_state);
    result
}

#[no_mangle]
pub extern "C" fn sched_yield() -> i32 {
    let _ = switch_current(ThreadState::Ready);
    0
}

#[no_mangle]
pub extern "C" fn pthread_yield() -> i32 {
    sched_yield()
}

#[no_mangle]
pub extern "C" fn pthread_exit(result: *mut c_void) -> ! {
    exit_current(result)
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_init(mutex: *mut c_void, attributes: *const c_void) -> i32 {
    if mutex.is_null() || !attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        (*mutex.cast::<PthreadMutex>())
            .owner
            .store(0, Ordering::Relaxed)
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_destroy(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe {
        (*mutex.cast::<PthreadMutex>())
            .owner
            .load(Ordering::Relaxed)
    };
    if owner != 0 {
        EBUSY
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_lock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = current_thread_id();
    if current_id == 0 {
        return EAGAIN;
    }
    loop {
        match owner.compare_exchange(0, current_id, Ordering::Acquire, Ordering::Relaxed) {
            Ok(_) => return 0,
            Err(existing) if existing == current_id => return EDEADLK,
            Err(_) if !switch_current(ThreadState::Ready) => return EDEADLK,
            Err(_) => {}
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_trylock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = current_thread_id();
    if current_id == 0 {
        return EAGAIN;
    }
    match owner.compare_exchange(0, current_id, Ordering::Acquire, Ordering::Relaxed) {
        Ok(_) => 0,
        Err(_) => EBUSY,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_mutex_unlock(mutex: *mut c_void) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let owner = unsafe { &(*mutex.cast::<PthreadMutex>()).owner };
    let current_id = current_thread_id();
    if current_id == 0 {
        return EPERM;
    }
    match owner.compare_exchange(current_id, 0, Ordering::Release, Ordering::Relaxed) {
        Ok(_) => 0,
        Err(_) => EPERM,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_init(
    condition: *mut c_void,
    attributes: *const c_void,
) -> i32 {
    if condition.is_null() || !attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .store(0, Ordering::Relaxed)
    };
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_destroy(condition: *mut c_void) -> i32 {
    if condition.is_null() {
        EINVAL
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> i32 {
    if condition.is_null() || mutex.is_null() {
        return EINVAL;
    }
    let generation = unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .load(Ordering::Acquire)
    };
    let unlock_status = unsafe { pthread_mutex_unlock(mutex) };
    if unlock_status != 0 {
        return unlock_status;
    }

    let condition_generation = unsafe { &(*condition.cast::<PthreadCondition>()).generation };
    while condition_generation.load(Ordering::Acquire) == generation {
        if !switch_current(ThreadState::Ready) {
            let lock_status = unsafe { pthread_mutex_lock(mutex) };
            return if lock_status == 0 {
                EDEADLK
            } else {
                lock_status
            };
        }
    }
    unsafe { pthread_mutex_lock(mutex) }
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_signal(condition: *mut c_void) -> i32 {
    if condition.is_null() {
        return EINVAL;
    }
    unsafe {
        (*condition.cast::<PthreadCondition>())
            .generation
            .fetch_add(1, Ordering::Release);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_cond_broadcast(condition: *mut c_void) -> i32 {
    unsafe { pthread_cond_signal(condition) }
}
