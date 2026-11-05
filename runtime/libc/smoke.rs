/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_void};
use core::mem::MaybeUninit;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::errno::{EINVAL, ENOENT};
use crate::kernel::arch::traits::TimerArchitecture as _;

use super::filesystem::CStat;
use super::stdio::KzFile;

const O_RDONLY: i32 = 0;
const O_RDWR: i32 = 2;
const O_CREAT: i32 = 0o100;
const O_EXCL: i32 = 0o200;
const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const MAP_PRIVATE: i32 = 2;
const MAP_ANONYMOUS: i32 = 0x20;
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;
const SEEK_SET: i32 = 0;

#[repr(C)]
struct PthreadAttributes {
    stack_size: usize,
    detach_state: i32,
    stack_address: *mut c_void,
}

#[repr(C)]
struct PthreadMutex {
    owner: usize,
}

#[repr(C)]
struct PthreadCondition {
    generation: usize,
}

type StartRoutine = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

static mut SMOKE_MUTEX: PthreadMutex = PthreadMutex { owner: 0 };
static mut SMOKE_CONDITION: PthreadCondition = PthreadCondition { generation: 0 };
static SMOKE_READY: AtomicBool = AtomicBool::new(false);
static SMOKE_FINISHED: AtomicUsize = AtomicUsize::new(0);
static SMOKE_PREEMPTION_STAGE: AtomicUsize = AtomicUsize::new(0);
static SMOKE_CAPACITY_GATE: AtomicBool = AtomicBool::new(false);
static SMOKE_CAPACITY_FINISHED: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(pointer: *mut c_void);
    fn mmap(
        address: *mut c_void,
        length: usize,
        protection: i32,
        flags: i32,
        file_descriptor: i32,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(address: *mut c_void, length: usize) -> i32;
    fn printf(format: *const c_char, ...) -> i32;
    fn puts(text: *const c_char) -> i32;
    fn putchar(character: i32) -> i32;
    fn pthread_attr_init(attributes: *mut PthreadAttributes) -> i32;
    fn pthread_attr_destroy(attributes: *mut PthreadAttributes) -> i32;
    fn pthread_attr_setdetachstate(attributes: *mut PthreadAttributes, state: i32) -> i32;
    fn pthread_attr_setstacksize(attributes: *mut PthreadAttributes, stack_size: usize) -> i32;
    fn pthread_create(
        thread: *mut usize,
        attributes: *const c_void,
        start: Option<StartRoutine>,
        argument: *mut c_void,
    ) -> i32;
    fn pthread_getattr_np(thread: usize, attributes: *mut PthreadAttributes) -> i32;
    fn pthread_join(thread: usize, result: *mut *mut c_void) -> i32;
    fn pthread_detach(thread: usize) -> i32;
    fn pthread_mutex_lock(mutex: *mut c_void) -> i32;
    fn pthread_mutex_unlock(mutex: *mut c_void) -> i32;
    fn pthread_mutex_destroy(mutex: *mut c_void) -> i32;
    fn pthread_cond_signal(condition: *mut c_void) -> i32;
    fn pthread_cond_wait(condition: *mut c_void, mutex: *mut c_void) -> i32;
    fn pthread_cond_destroy(condition: *mut c_void) -> i32;
    fn pthread_yield() -> i32;
}

#[no_mangle]
pub unsafe extern "C" fn kz_c_stdio_smoke() -> i32 {
    if unsafe {
        printf(
            b"kz: C stdio printf %d %s 0x%x\n\0".as_ptr().cast(),
            42,
            b"hello\0".as_ptr(),
            42u32,
        )
    } < 0
    {
        return -1;
    }
    if unsafe { puts(b"kz: C stdio puts works\0".as_ptr().cast()) } == -1 {
        return -1;
    }
    if unsafe { putchar(b'!' as i32) } == -1 || unsafe { putchar(b'\n' as i32) } == -1 {
        return -1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn kz_c_malloc_smoke() -> i32 {
    let first = unsafe { malloc(37) }.cast::<u8>();
    let second = unsafe { malloc(91) }.cast::<u8>();
    if first.is_null()
        || second.is_null()
        || first == second
        || first as usize % 16 != 0
        || second as usize % 16 != 0
    {
        return -1;
    }
    unsafe {
        write_volatile(first, 0x5a);
        write_volatile(first.add(36), 0xa5);
        write_volatile(second, 0x3c);
        free(first.cast());
    }
    let reused = unsafe { malloc(37) }.cast::<u8>();
    if reused != first {
        return -1;
    }
    unsafe {
        write_volatile(reused, 0x11);
        free(reused.cast());
        free(second.cast());
        free(core::ptr::null_mut());
    }
    let coalesced = unsafe { malloc(8192) };
    if coalesced.is_null() {
        return -1;
    }
    unsafe { free(coalesced) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn kz_c_mmap_smoke() -> i32 {
    let length = 2 * 4096;
    let mut memory = unsafe {
        mmap(
            core::ptr::null_mut(),
            length,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if memory == MAP_FAILED || memory as usize % 4096 != 0 {
        return -1;
    }
    let bytes = memory.cast::<u8>();
    if unsafe { read_volatile(bytes) } != 0 || unsafe { read_volatile(bytes.add(length - 1)) } != 0
    {
        return -1;
    }
    unsafe {
        write_volatile(bytes, 0x5a);
        write_volatile(bytes.add(length - 1), 0xa5);
    }
    if unsafe { munmap(memory, length) } != 0 {
        return -1;
    }
    memory = unsafe {
        mmap(
            core::ptr::null_mut(),
            length,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    if memory == MAP_FAILED {
        return -1;
    }
    let bytes = memory.cast::<u8>();
    if unsafe { read_volatile(bytes) } != 0 || unsafe { read_volatile(bytes.add(length - 1)) } != 0
    {
        return -1;
    }
    if unsafe { munmap(memory, length) } != 0 {
        return -1;
    }
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(0) };
    if unsafe {
        mmap(
            core::ptr::null_mut(),
            0,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS,
            -1,
            0,
        )
    } != MAP_FAILED
        || unsafe { crate::kernel::scheduler::thread::__errno_location().read() } != EINVAL
    {
        return -1;
    }
    0
}

unsafe extern "C" fn smoke_worker(argument: *mut c_void) -> *mut c_void {
    let mutex = core::ptr::addr_of_mut!(SMOKE_MUTEX).cast();
    if unsafe { pthread_mutex_lock(mutex) } != 0 {
        return core::ptr::null_mut();
    }
    SMOKE_READY.store(true, Ordering::Release);
    let signal_status = unsafe {
        pthread_cond_signal(core::ptr::addr_of_mut!(SMOKE_CONDITION).cast())
    };
    let unlock_status = unsafe { pthread_mutex_unlock(mutex) };
    if signal_status == 0 && unlock_status == 0 {
        argument
    } else {
        core::ptr::null_mut()
    }
}

unsafe extern "C" fn smoke_completion_worker(argument: *mut c_void) -> *mut c_void {
    SMOKE_FINISHED.fetch_add(1, Ordering::AcqRel);
    argument
}

unsafe extern "C" fn smoke_preemption_worker(_argument: *mut c_void) -> *mut c_void {
    SMOKE_PREEMPTION_STAGE.store(1, Ordering::Release);
    while SMOKE_PREEMPTION_STAGE.load(Ordering::Acquire) < 2 {
        core::hint::spin_loop();
    }
    SMOKE_PREEMPTION_STAGE.store(3, Ordering::Release);
    core::ptr::null_mut()
}

fn wait_for_preemption_stage(stage: usize) -> bool {
    let start_ns = crate::kernel::arch::TimerArchitecture::now_ns();
    for _ in 0..50_000_000 {
        if SMOKE_PREEMPTION_STAGE.load(Ordering::Acquire) >= stage {
            return true;
        }
        if start_ns.is_some_and(|start| {
            crate::kernel::arch::TimerArchitecture::now_ns()
                .is_some_and(|now| now.wrapping_sub(start) >= 250_000_000)
        }) {
            return false;
        }
        core::hint::spin_loop();
    }
    false
}

unsafe extern "C" fn smoke_capacity_worker(argument: *mut c_void) -> *mut c_void {
    while !SMOKE_CAPACITY_GATE.load(Ordering::Acquire) {
        unsafe { pthread_yield() };
    }
    SMOKE_CAPACITY_FINISHED.fetch_add(1, Ordering::Release);
    argument
}

#[no_mangle]
pub unsafe extern "C" fn kz_c_pthread_smoke() -> i32 {
    const CUSTOM_STACK_SIZE: usize = 32 * 1024;
    const CAPACITY_THREADS: usize = 8;

    let mut attributes = PthreadAttributes {
        stack_size: 0,
        detach_state: 0,
        stack_address: core::ptr::null_mut(),
    };
    if unsafe { pthread_attr_init(&mut attributes) } != 0
        || unsafe { pthread_attr_setdetachstate(&mut attributes, 0) } != 0
        || unsafe { pthread_attr_setstacksize(&mut attributes, CUSTOM_STACK_SIZE) } != 0
    {
        return -1;
    }
    SMOKE_PREEMPTION_STAGE.store(0, Ordering::Release);
    let mut preemption_thread = 0usize;
    if unsafe {
        pthread_create(
            &mut preemption_thread,
            core::ptr::addr_of!(attributes).cast(),
            Some(smoke_preemption_worker),
            core::ptr::null_mut(),
        )
    } != 0
        || !wait_for_preemption_stage(1)
    {
        return -1;
    }
    let mut preemption_attributes = PthreadAttributes {
        stack_size: 0,
        detach_state: 0,
        stack_address: core::ptr::null_mut(),
    };
    if unsafe { pthread_getattr_np(preemption_thread, &mut preemption_attributes) } != 0
        || preemption_attributes.stack_size != CUSTOM_STACK_SIZE
    {
        return -1;
    }
    SMOKE_PREEMPTION_STAGE.store(2, Ordering::Release);
    if !wait_for_preemption_stage(3) {
        return -1;
    }
    let mut preemption_result = core::ptr::null_mut();
    if unsafe { pthread_join(preemption_thread, &mut preemption_result) } != 0
        || !preemption_result.is_null()
    {
        return -1;
    }

    SMOKE_CAPACITY_GATE.store(false, Ordering::Release);
    SMOKE_CAPACITY_FINISHED.store(0, Ordering::Release);
    let mut capacity_threads = [0usize; CAPACITY_THREADS];
    let mut created_threads = 0;
    while created_threads < CAPACITY_THREADS {
        let argument = (created_threads + 1) as *mut c_void;
        let create_status = unsafe {
            pthread_create(
                &mut capacity_threads[created_threads],
                core::ptr::addr_of!(attributes).cast(),
                Some(smoke_capacity_worker),
                argument,
            )
        };
        if create_status != 0 {
            SMOKE_CAPACITY_GATE.store(true, Ordering::Release);
            for thread in capacity_threads.iter().take(created_threads) {
                unsafe { pthread_join(*thread, core::ptr::null_mut()) };
            }
            return -1;
        }
        created_threads += 1;
    }
    SMOKE_CAPACITY_GATE.store(true, Ordering::Release);
    for (index, thread) in capacity_threads.iter().copied().enumerate() {
        let mut result = core::ptr::null_mut();
        if unsafe { pthread_join(thread, &mut result) } != 0
            || result != (index + 1) as *mut c_void
        {
            return -1;
        }
    }
    if SMOKE_CAPACITY_FINISHED.load(Ordering::Acquire) != CAPACITY_THREADS {
        return -1;
    }

    SMOKE_READY.store(false, Ordering::Release);
    let mut thread = 0usize;
    if unsafe {
        pthread_create(
            &mut thread,
            core::ptr::addr_of!(attributes).cast(),
            Some(smoke_worker),
            42usize as *mut c_void,
        )
    } != 0
    {
        return -1;
    }
    let mutex = core::ptr::addr_of_mut!(SMOKE_MUTEX).cast();
    if unsafe { pthread_mutex_lock(mutex) } != 0 {
        return -1;
    }
    while !SMOKE_READY.load(Ordering::Acquire) {
        if unsafe {
            pthread_cond_wait(core::ptr::addr_of_mut!(SMOKE_CONDITION).cast(), mutex)
        } != 0
        {
            return -1;
        }
    }
    if unsafe { pthread_mutex_unlock(mutex) } != 0 {
        return -1;
    }
    let mut result = core::ptr::null_mut();
    if unsafe { pthread_join(thread, &mut result) } != 0 || result != 42usize as *mut c_void {
        return -1;
    }
    if unsafe { pthread_mutex_destroy(mutex) } != 0
        || unsafe { pthread_cond_destroy(core::ptr::addr_of_mut!(SMOKE_CONDITION).cast()) } != 0
    {
        return -1;
    }

    SMOKE_FINISHED.store(0, Ordering::Release);
    if unsafe { pthread_attr_setdetachstate(&mut attributes, 1) } != 0
        || unsafe {
            pthread_create(
                &mut thread,
                core::ptr::addr_of!(attributes).cast(),
                Some(smoke_completion_worker),
                84usize as *mut c_void,
            )
        } != 0
    {
        return -1;
    }
    while SMOKE_FINISHED.load(Ordering::Acquire) < 1 {
        unsafe { pthread_yield() };
    }
    if unsafe { pthread_attr_setdetachstate(&mut attributes, 0) } != 0
        || unsafe {
            pthread_create(
                &mut thread,
                core::ptr::addr_of!(attributes).cast(),
                Some(smoke_completion_worker),
                126usize as *mut c_void,
            )
        } != 0
        || unsafe { pthread_detach(thread) } != 0
    {
        return -1;
    }
    while SMOKE_FINISHED.load(Ordering::Acquire) < 2 {
        unsafe { pthread_yield() };
    }
    if unsafe {
        pthread_create(
            &mut thread,
            core::ptr::addr_of!(attributes).cast(),
            Some(smoke_completion_worker),
            168usize as *mut c_void,
        )
    } != 0
        || unsafe { pthread_join(thread, &mut result) } != 0
        || result != 168usize as *mut c_void
    {
        return -1;
    }
    unsafe { pthread_attr_destroy(&mut attributes) }
}

unsafe fn cleanup_filesystem_smoke(
    file_descriptor: i32,
    stream: *mut KzFile,
    path: *const c_char,
    directory: *const c_char,
    child_path: *const c_char,
) {
    if !stream.is_null() {
        unsafe { super::stdio::fclose(stream) };
    }
    if file_descriptor >= 0 {
        super::filesystem::close(file_descriptor);
    }
    unsafe {
        super::filesystem::unlink(child_path);
        super::filesystem::rmdir(directory);
        super::filesystem::unlink(path);
    }
}

#[no_mangle]
pub unsafe extern "C" fn kz_c_filesystem_smoke() -> i32 {
    let path = b"/kz-heap-fs-smoke\0";
    let directory = b"/kz-heap-fs-dir\0";
    let child_path = b"/kz-heap-fs-dir/entry\0";
    let payload = b"heap-backed file";
    let mut buffer = [0u8; 32];
    let mut status = MaybeUninit::<CStat>::uninit();
    let mut stream = core::ptr::null_mut();
    let mut file_descriptor = unsafe {
        super::filesystem::kz_rust_open(path.as_ptr().cast(), O_CREAT | O_EXCL | O_RDWR, 0o600)
    };
    if file_descriptor < 0
        || unsafe {
            super::filesystem::write(file_descriptor, payload.as_ptr().cast(), payload.len())
        } != payload.len() as isize
        || super::filesystem::lseek(file_descriptor, 0, SEEK_SET) != 0
        || unsafe {
            super::filesystem::read(file_descriptor, buffer.as_mut_ptr().cast(), payload.len())
        } != payload.len() as isize
        || buffer[..payload.len()] != payload[..]
        || unsafe { super::filesystem::fstat64(file_descriptor, status.as_mut_ptr().cast()) } != 0
        || unsafe { status.assume_init_ref().size } != payload.len() as i64
    {
        unsafe {
            cleanup_filesystem_smoke(
                file_descriptor,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }
    if super::filesystem::close(file_descriptor) != 0 {
        unsafe {
            cleanup_filesystem_smoke(
                -1,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }
    stream = unsafe { super::stdio::fopen64(path.as_ptr().cast(), b"a+\0".as_ptr().cast()) };
    if stream.is_null()
        || unsafe { super::format::fwrite(b"!".as_ptr().cast(), 1, 1, stream) } != 1
        || unsafe { super::stdio::fseek(stream, 0, SEEK_SET) } != 0
        || unsafe { super::stdio::fgetc(stream) } != b'h' as i32
        || unsafe { super::stdio::ungetc(b'H' as i32, stream) } != b'H' as i32
        || unsafe { super::stdio::ftello64(stream) } != 0
        || unsafe { super::stdio::fgetc(stream) } != b'H' as i32
        || unsafe { super::stdio::fseek(stream, 0, SEEK_SET) } != 0
        || unsafe { super::stdio::fread(buffer.as_mut_ptr().cast(), 1, payload.len() + 1, stream) }
            != payload.len() + 1
        || buffer[..payload.len() + 1] != b"heap-backed file!"[..]
        || unsafe { super::stdio::ftello64(stream) } != (payload.len() + 1) as i64
    {
        unsafe {
            cleanup_filesystem_smoke(
                -1,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }
    let temporary_stream = stream;
    stream = core::ptr::null_mut();
    if unsafe { super::stdio::fclose(temporary_stream) } != 0 {
        unsafe {
            cleanup_filesystem_smoke(
                -1,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }

    if unsafe { super::filesystem::stat64(path.as_ptr().cast(), status.as_mut_ptr().cast()) } != 0
        || unsafe { status.assume_init_ref().size } != (payload.len() + 1) as i64
        || unsafe { super::filesystem::mkdir(directory.as_ptr().cast(), 0o700) } != 0
    {
        unsafe {
            cleanup_filesystem_smoke(
                -1,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }
    file_descriptor = unsafe {
        super::filesystem::kz_rust_open(
            child_path.as_ptr().cast(),
            O_CREAT | O_EXCL | O_RDWR,
            0o600,
        )
    };
    if file_descriptor < 0
        || super::filesystem::close(file_descriptor) != 0
        || unsafe { super::filesystem::unlink(child_path.as_ptr().cast()) } != 0
        || unsafe { super::filesystem::rmdir(directory.as_ptr().cast()) } != 0
    {
        unsafe {
            cleanup_filesystem_smoke(
                file_descriptor,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }
    stream = unsafe { super::stdio::tmpfile64() };
    if stream.is_null()
        || unsafe { super::stdio::fputs(b"tmp\0".as_ptr().cast(), stream) } == -1
        || unsafe { super::stdio::fseek(stream, 0, SEEK_SET) } != 0
        || unsafe { super::stdio::fread(buffer.as_mut_ptr().cast(), 1, 3, stream) } != 3
        || buffer[..3] != b"tmp"[..]
    {
        unsafe {
            cleanup_filesystem_smoke(
                -1,
                stream,
                path.as_ptr().cast(),
                directory.as_ptr().cast(),
                child_path.as_ptr().cast(),
            );
        }
        return -1;
    }
    let temporary_stream = stream;
    if unsafe { super::stdio::fclose(temporary_stream) } != 0
        || unsafe { super::filesystem::unlink(path.as_ptr().cast()) } != 0
    {
        return -1;
    }
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(0) };
    let missing = unsafe { super::filesystem::kz_rust_open(path.as_ptr().cast(), O_RDONLY, 0) };
    if missing != -1
        || unsafe { crate::kernel::scheduler::thread::__errno_location().read() } != ENOENT
    {
        return -1;
    }
    0
}
