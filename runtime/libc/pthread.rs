use core::ffi::c_void;

use crate::kernel::scheduler::thread::ThreadStack;

const JOINABLE: i32 = 0;
const DETACHED: i32 = 1;

#[no_mangle]
pub extern "C" fn pthread_self() -> usize {
    crate::kernel::scheduler::thread::current_thread_id()
}

#[no_mangle]
pub extern "C" fn pthread_equal(first: usize, second: usize) -> i32 {
    (first == second) as i32
}

#[no_mangle]
pub extern "C" fn sched_yield() -> i32 {
    crate::kernel::scheduler::thread::yield_current()
}

#[no_mangle]
pub extern "C" fn pthread_yield() -> i32 {
    sched_yield()
}

#[no_mangle]
pub extern "C" fn pthread_exit(result: *mut c_void) -> ! {
    crate::kernel::scheduler::thread::exit_current(result)
}

macro_rules! export_pthread {
    ($name:ident($($argument:ident: $argument_type:ty),* $(,)?) -> $result:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($argument: $argument_type),*) -> $result {
            unsafe { crate::kernel::scheduler::thread::$name($($argument),*) }
        }
    };
}


pub(crate) unsafe fn pthread_key_create(
    key: *mut u32,
    destructor: Option<unsafe extern "C" fn(*mut c_void)>,
) -> i32 {
    if key.is_null() {
        return EINVAL;
    }
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let keys = addr_of_mut!(TLS_KEYS).cast::<TlsKey>();
    for index in 0..MAX_TLS_KEYS {
        let slot = unsafe { &mut *keys.add(index) };
        if !slot.active {
            *slot = TlsKey {
                active: true,
                destructor,
            };
            unsafe { key.write((index + 1) as u32) };
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return 0;
        }
    }
    crate::kernel::arch::interrupts::restore(interrupt_state);
    EAGAIN
}

pub(crate) unsafe fn pthread_key_delete(key: u32) -> i32 {
    if key == 0 || key as usize > MAX_TLS_KEYS {
        return EINVAL;
    }
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let index = key as usize - 1;
    let keys = addr_of_mut!(TLS_KEYS).cast::<TlsKey>();
    let slot = unsafe { &mut *keys.add(index) };
    if !slot.active {
        crate::kernel::arch::interrupts::restore(interrupt_state);
        return EINVAL;
    }
    *slot = TlsKey::EMPTY;
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    for thread_index in 0..MAX_THREADS {
        unsafe { (*threads.add(thread_index)).tls[index] = core::ptr::null_mut() };
    }
    crate::kernel::arch::interrupts::restore(interrupt_state);
    0
}

pub(crate) unsafe fn pthread_getspecific(key: u32) -> *mut c_void {
    if key == 0 || key as usize > MAX_TLS_KEYS {
        return core::ptr::null_mut();
    }
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let index = key as usize - 1;
    let value = unsafe {
        let keys = addr_of_mut!(TLS_KEYS).cast::<TlsKey>();
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        if !INITIALIZED || !(*keys.add(index)).active {
            core::ptr::null_mut()
        } else {
            (*threads.add(CURRENT_SLOT)).tls[index]
        }
    };
    crate::kernel::arch::interrupts::restore(interrupt_state);
    value
}

pub(crate) unsafe fn pthread_setspecific(key: u32, value: *const c_void) -> i32 {
    if key == 0 || key as usize > MAX_TLS_KEYS {
        return EINVAL;
    }
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let index = key as usize - 1;
    let keys = addr_of_mut!(TLS_KEYS).cast::<TlsKey>();
    if !INITIALIZED || !unsafe { (*keys.add(index)).active } {
        crate::kernel::arch::interrupts::restore(interrupt_state);
        return EINVAL;
    }
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    unsafe { (*threads.add(CURRENT_SLOT)).tls[index] = value.cast_mut() };
    crate::kernel::arch::interrupts::restore(interrupt_state);
    0
}

pub(crate) unsafe fn pthread_once(
    once: *mut u32,
    initializer: Option<unsafe extern "C" fn()>,
) -> i32 {
    if once.is_null() || initializer.is_none() {
        return EINVAL;
    }
    let state = unsafe { &*once.cast::<core::sync::atomic::AtomicU32>() };
    loop {
        match state.load(Ordering::Acquire) {
            2 => return 0,
            0 => {
                if state
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    unsafe { initializer.unwrap()() };
                    state.store(2, Ordering::Release);
                    return 0;
                }
            }
            _ => {
                if !switch_current(ThreadState::Ready) {
                    core::hint::spin_loop();
                }
            }
        }
    }
}

pub(crate) unsafe fn pthread_getschedparam(
    thread_id: usize,
    policy: *mut i32,
    parameters: *mut c_void,
) -> i32 {
    if policy.is_null() || parameters.is_null() {
        return EINVAL;
    }
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    if unsafe { find_thread(thread_id) }.is_none() {
        crate::kernel::arch::interrupts::restore(interrupt_state);
        return ESRCH;
    }
    unsafe {
        policy.write(0);
        parameters.cast::<i32>().write(0);
    }
    crate::kernel::arch::interrupts::restore(interrupt_state);
    0
}

pub(crate) unsafe fn pthread_attr_init(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size: 0,
                detach_state: JOINABLE,
                stack_address: core::ptr::null_mut(),
            });
    }
    0
}

pub(crate) unsafe fn pthread_attr_destroy(attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size: 0,
                detach_state: JOINABLE,
                stack_address: core::ptr::null_mut(),
            });
    }
    0
}

pub(crate) unsafe fn pthread_attr_setdetachstate(
    attributes: *mut c_void,
    detach_state: i32,
) -> i32 {
    if attributes.is_null() || !matches!(detach_state, JOINABLE | DETACHED) {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).detach_state = detach_state };
    0
}

pub(crate) unsafe fn pthread_attr_getdetachstate(
    attributes: *const c_void,
    detach_state: *mut i32,
) -> i32 {
    if attributes.is_null() || detach_state.is_null() {
        return EINVAL;
    }
    unsafe { detach_state.write((*attributes.cast::<ThreadAttributes>()).detach_state) };
    0
}

pub(crate) unsafe fn pthread_attr_setstacksize(attributes: *mut c_void, stack_size: usize) -> i32 {
    if attributes.is_null() || stack_size != STACK_SIZE {
        return EINVAL;
    }
    unsafe { (*attributes.cast::<ThreadAttributes>()).stack_size = stack_size };
    0
}

pub(crate) unsafe fn pthread_attr_getstacksize(
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

pub(crate) unsafe fn pthread_attr_getstack(
    attributes: *const c_void,
    stack_address: *mut *mut c_void,
    stack_size: *mut usize,
) -> i32 {
    if attributes.is_null() || stack_address.is_null() || stack_size.is_null() {
        return EINVAL;
    }
    let attributes = unsafe { &*attributes.cast::<ThreadAttributes>() };
    unsafe {
        stack_address.write(attributes.stack_address);
        stack_size.write(if attributes.stack_size == 0 {
            STACK_SIZE
        } else {
            attributes.stack_size
        });
    }
    0
}

pub(crate) unsafe fn pthread_getattr_np(thread_id: usize, attributes: *mut c_void) -> i32 {
    if attributes.is_null() {
        return EINVAL;
    }
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let Some(slot) = (unsafe { find_thread(thread_id) }) else {
        crate::kernel::arch::interrupts::restore(interrupt_state);
        return ESRCH;
    };
    let threads = addr_of_mut!(THREADS).cast::<Thread>();
    let thread = unsafe { *threads.add(slot) };
    let (stack_address, stack_size) = ThreadStack::bounds(slot);
    unsafe {
        attributes
            .cast::<ThreadAttributes>()
            .write(ThreadAttributes {
                stack_size,
                detach_state: if thread.detached { DETACHED } else { JOINABLE },
                stack_address,
            });
    }
    crate::kernel::arch::interrupts::restore(interrupt_state);
    0
}

pub(crate) unsafe fn pthread_kill(thread_id: usize, signal_number: i32) -> i32 {
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let exists = unsafe { find_thread(thread_id) }.is_some();
    crate::kernel::arch::interrupts::restore(interrupt_state);
    if !exists {
        return ESRCH;
    }
    if signal_number == 0 {
        0
    } else {
        ENOSYS
    }
}

pub(crate) unsafe fn pthread_create(
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

    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let id = unsafe {
        if !INITIALIZED || NEXT_THREAD_ID == usize::MAX {
            crate::kernel::arch::interrupts::restore(interrupt_state);
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
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return EAGAIN;
        };

        let id = NEXT_THREAD_ID;
        NEXT_THREAD_ID += 1;
        let stack_pointer = crate::kernel::arch::initialize_thread_stack(
            ThreadStack::slice_at(slot),
            thread_trampoline,
        );
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
    crate::kernel::arch::interrupts::restore(interrupt_state);
    0
}

pub(crate) unsafe fn pthread_join(thread_id: usize, result: *mut *mut c_void) -> i32 {
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let (current_id, target_slot, target_state) = unsafe {
        if !INITIALIZED {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        }
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let current_id = (*threads.add(CURRENT_SLOT)).id;
        if thread_id == current_id {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return EDEADLK;
        }
        let Some(target_slot) = find_thread(thread_id) else {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        };
        let target = threads.add(target_slot);
        if (*target).detached || (*target).joiner != 0 {
            crate::kernel::arch::interrupts::restore(interrupt_state);
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
        crate::kernel::arch::interrupts::restore(interrupt_state);
        if !result.is_null() {
            unsafe { result.write(value) };
        }
        return 0;
    }

    unsafe {
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        (*threads.add(target_slot)).joiner = current_id;
    }
    crate::kernel::arch::interrupts::restore(interrupt_state);

    if !switch_current(ThreadState::Blocked) {
        let interrupt_state = crate::kernel::arch::interrupts::disable();
        unsafe {
            let threads = addr_of_mut!(THREADS).cast::<Thread>();
            if let Some(target_slot) = find_thread(thread_id) {
                (*threads.add(target_slot)).joiner = 0;
            }
        }
        crate::kernel::arch::interrupts::restore(interrupt_state);
        return EDEADLK;
    }

    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let value = unsafe {
        let Some(target_slot) = find_thread(thread_id) else {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        };
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let target = threads.add(target_slot);
        if (*target).state != ThreadState::Exited {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return EDEADLK;
        }
        let value = (*target).result;
        target.write(Thread::EMPTY);
        value
    };
    crate::kernel::arch::interrupts::restore(interrupt_state);
    if !result.is_null() {
        unsafe { result.write(value) };
    }
    0
}

pub(crate) unsafe fn pthread_detach(thread_id: usize) -> i32 {
    let interrupt_state = crate::kernel::arch::interrupts::disable();
    let result = unsafe {
        let Some(slot) = find_thread(thread_id) else {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return ESRCH;
        };
        let threads = addr_of_mut!(THREADS).cast::<Thread>();
        let thread = threads.add(slot);
        if (*thread).detached || (*thread).joiner != 0 {
            crate::kernel::arch::interrupts::restore(interrupt_state);
            return EINVAL;
        }
        if (*thread).state == ThreadState::Exited {
            thread.write(Thread::EMPTY);
        } else {
            (*thread).detached = true;
        }
        0
    };
    crate::kernel::arch::interrupts::restore(interrupt_state);
    result
}

pub(crate) fn yield_current() -> i32 {
    let _ = switch_current(ThreadState::Ready);
    0
}

pub(crate) unsafe fn pthread_mutex_init(mutex: *mut c_void, attributes: *const c_void) -> i32 {
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

pub(crate) unsafe fn pthread_mutex_destroy(mutex: *mut c_void) -> i32 {
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

pub(crate) unsafe fn pthread_mutex_lock(mutex: *mut c_void) -> i32 {
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

pub(crate) unsafe fn pthread_mutex_trylock(mutex: *mut c_void) -> i32 {
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

pub(crate) unsafe fn pthread_mutex_unlock(mutex: *mut c_void) -> i32 {
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

pub(crate) unsafe fn pthread_cond_init(condition: *mut c_void, attributes: *const c_void) -> i32 {
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

pub(crate) unsafe fn pthread_cond_destroy(condition: *mut c_void) -> i32 {
    if condition.is_null() {
        EINVAL
    } else {
        0
    }
}

pub(crate) unsafe fn pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> i32 {
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

pub(crate) unsafe fn pthread_cond_signal(condition: *mut c_void) -> i32 {
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

pub(crate) unsafe fn pthread_cond_broadcast(condition: *mut c_void) -> i32 {
    unsafe { pthread_cond_signal(condition) }
}
