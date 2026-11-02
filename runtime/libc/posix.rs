/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};

use super::variadic::{self, VA_INT, VA_POINTER, VA_UINT, VA_ULONG};

const EAGAIN: i32 = 11;
const EBUSY: i32 = 16;
const EBADF: i32 = 9;
const EFAULT: i32 = 14;
const EINVAL: i32 = 22;
const EIO: i32 = 5;
const ENOSYS: i32 = 38;
const ENOMEM: i32 = 12;
const ERANGE: i32 = 34;
const EOVERFLOW: i32 = 75;

const CLOCK_REALTIME: i32 = 0;
const CLOCK_MONOTONIC: i32 = 1;
const CLOCK_MONOTONIC_RAW: i32 = 4;
const SIGSET_SIZE: usize = 128;
const PR_SET_NAME: i32 = 15;
const PR_GET_NAME: i32 = 16;
const PR_SET_VMA: i32 = 0x5356_4d41;
const FUTEX_WAIT: i32 = 0;
const FUTEX_WAKE: i32 = 1;
const FUTEX_PRIVATE_FLAG: i32 = 128;
const SYS_GETPID: i64 = 39;
const SYS_SCHED_YIELD: i64 = 24;
const SYS_FUTEX: i64 = 202;
const SYS_GETTID: i64 = 186;
const SYS_CLOCK_GETTIME: i64 = 228;
const SC_PAGESIZE: i32 = 30;
const SC_NPROCESSORS_CONF: i32 = 83;
const SC_NPROCESSORS_ONLN: i32 = 84;
const SC_CLK_TCK: i32 = 2;

#[repr(C)]
struct Timespec {
    seconds: c_long,
    nanoseconds: c_long,
}

#[repr(C)]
struct Tm {
    seconds: i32,
    minutes: i32,
    hours: i32,
    month_day: i32,
    month: i32,
    year: i32,
    week_day: i32,
    year_day: i32,
    daylight: i32,
    offset: c_long,
    zone: *const c_char,
}

#[repr(C)]
struct Timeval {
    seconds: c_long,
    microseconds: c_long,
}

#[repr(C)]
struct Utsname {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domainname: [u8; 65],
}

unsafe extern "C" {
    fn kz_host_time_ns() -> u64;
    fn kz_host_wall_time_ms() -> i64;
    fn pthread_self() -> usize;
    fn pthread_yield() -> i32;
    fn __cxa_finalize(dso: *mut c_void);
    fn fflush(stream: *mut c_void) -> i32;
    fn mmap(
        address: *mut c_void,
        length: usize,
        protection: i32,
        flags: i32,
        file_descriptor: i32,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(address: *mut c_void, length: usize) -> i32;
}

pub(crate) fn set_errno(error: i32) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

#[no_mangle]
pub unsafe extern "C" fn clock_gettime(clock_id: i32, output: *mut Timespec) -> i32 {
    if output.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let (seconds, nanoseconds) = match clock_id {
        CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW => {
            let now = unsafe { kz_host_time_ns() };
            (now / 1_000_000_000, now % 1_000_000_000)
        }
        CLOCK_REALTIME => {
            let milliseconds = unsafe { kz_host_wall_time_ms() }.max(0) as u64;
            (milliseconds / 1000, milliseconds % 1000 * 1_000_000)
        }
        _ => {
            set_errno(EINVAL);
            return -1;
        }
    };
    unsafe {
        output.write(Timespec {
            seconds: seconds as c_long,
            nanoseconds: nanoseconds as c_long,
        });
    }
    0
}

#[no_mangle]
pub extern "C" fn getcwd(buffer: *mut c_char, size: usize) -> *mut c_char {
    if buffer.is_null() {
        set_errno(EFAULT);
        return core::ptr::null_mut();
    }
    if size < 2 {
        set_errno(ERANGE);
        return core::ptr::null_mut();
    }
    unsafe {
        buffer.write(b'/' as c_char);
        buffer.add(1).write(0);
    }
    buffer
}

#[no_mangle]
pub extern "C" fn sysconf(name: i32) -> isize {
    match name {
        SC_PAGESIZE => 4096,
        SC_NPROCESSORS_CONF | SC_NPROCESSORS_ONLN => 1,
        SC_CLK_TCK => 100,
        _ => {
            set_errno(EINVAL);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn __sysconf(name: i32) -> isize {
    sysconf(name)
}

#[no_mangle]
pub extern "C" fn getpagesize() -> isize {
    4096
}

#[no_mangle]
pub extern "C" fn getpid() -> i32 {
    1
}

#[no_mangle]
pub unsafe extern "C" fn sigfillset(set: *mut c_void) -> i32 {
    if set.is_null() {
        return EFAULT;
    }
    let bytes = set.cast::<u8>();
    for index in 0..SIGSET_SIZE {
        unsafe { write_volatile(bytes.add(index), 0xff) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigemptyset(set: *mut c_void) -> i32 {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let bytes = set.cast::<u8>();
    for index in 0..SIGSET_SIZE {
        unsafe { write_volatile(bytes.add(index), 0) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pthread_sigmask(
    how: i32,
    _set: *const c_void,
    old_set: *mut c_void,
) -> i32 {
    if !matches!(how, 0..=2) {
        return EINVAL;
    }
    if !old_set.is_null() {
        let bytes = old_set.cast::<u8>();
        for index in 0..SIGSET_SIZE {
            unsafe { write_volatile(bytes.add(index), 0) };
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn nanosleep(request: *const Timespec, remaining: *mut Timespec) -> i32 {
    if request.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    let request = unsafe { &*request };
    if request.seconds < 0 || request.nanoseconds < 0 || request.nanoseconds >= 1_000_000_000 {
        set_errno(EINVAL);
        return -1;
    }
    let now = unsafe { kz_host_time_ns() };
    let seconds = request.seconds as u64;
    let nanoseconds = request.nanoseconds as u64;
    let duration = seconds
        .checked_mul(1_000_000_000)
        .and_then(|seconds| seconds.checked_add(nanoseconds))
        .unwrap_or(u64::MAX);
    let deadline = now.saturating_add(duration);
    while unsafe { kz_host_time_ns() } < deadline {
        unsafe { pthread_yield() };
    }
    if !remaining.is_null() {
        unsafe {
            remaining.write(Timespec {
                seconds: 0,
                nanoseconds: 0,
            });
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn usleep(microseconds: u32) -> i32 {
    let request = Timespec {
        seconds: (microseconds / 1_000_000) as c_long,
        nanoseconds: (microseconds % 1_000_000) as c_long * 1000,
    };
    unsafe { nanosleep(&request, core::ptr::null_mut()) }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_prctl(option: i32, arguments: *mut c_void) -> i32 {
    match option {
        PR_SET_NAME => {
            let _ = unsafe { variadic::next(arguments, VA_POINTER) };
            0
        }
        PR_GET_NAME => {
            let name = unsafe { variadic::next(arguments, VA_POINTER) as *mut u8 };
            if name.is_null() {
                set_errno(EFAULT);
                return -1;
            }
            for (index, byte) in b"kz\0".iter().copied().enumerate() {
                unsafe { write_volatile(name.add(index), byte) };
            }
            0
        }
        PR_SET_VMA => {
            let _ = unsafe { variadic::next(arguments, VA_ULONG) };
            let _ = unsafe { variadic::next(arguments, VA_POINTER) };
            let _ = unsafe { variadic::next(arguments, VA_ULONG) };
            let _ = unsafe { variadic::next(arguments, VA_POINTER) };
            0
        }
        _ => {
            set_errno(EINVAL);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn _exit(_status: i32) -> ! {
    trap()
}

#[no_mangle]
pub unsafe extern "C" fn sem_init(semaphore: *mut c_void, process_shared: i32, value: u32) -> i32 {
    if semaphore.is_null() || process_shared != 0 || value > 0x7fff_ffff {
        return EINVAL;
    }
    unsafe { &*semaphore.cast::<AtomicU32>() }.store(value, Ordering::Release);
    0
}

#[no_mangle]
pub unsafe extern "C" fn sem_destroy(semaphore: *mut c_void) -> i32 {
    if semaphore.is_null() {
        EINVAL
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn sem_post(semaphore: *mut c_void) -> i32 {
    if semaphore.is_null() {
        return EINVAL;
    }
    let count = unsafe { &*semaphore.cast::<AtomicU32>() };
    let mut current = count.load(Ordering::Relaxed);
    loop {
        if current >= 0x7fff_ffff {
            return EOVERFLOW;
        }
        match count.compare_exchange_weak(
            current,
            current + 1,
            Ordering::Release,
            Ordering::Relaxed,
        ) {
            Ok(_) => return 0,
            Err(actual) => current = actual,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn sem_wait(semaphore: *mut c_void) -> i32 {
    if semaphore.is_null() {
        return EINVAL;
    }
    let count = unsafe { &*semaphore.cast::<AtomicU32>() };
    loop {
        let mut current = count.load(Ordering::Acquire);
        if current != 0
            && count
                .compare_exchange_weak(current, current - 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        {
            return 0;
        }
        unsafe { pthread_yield() };
    }
}

#[no_mangle]
pub unsafe extern "C" fn sem_trywait(semaphore: *mut c_void) -> i32 {
    if semaphore.is_null() {
        return EINVAL;
    }
    let count = unsafe { &*semaphore.cast::<AtomicU32>() };
    let mut current = count.load(Ordering::Acquire);
    while current != 0 {
        match count.compare_exchange_weak(
            current,
            current - 1,
            Ordering::Acquire,
            Ordering::Relaxed,
        ) {
            Ok(_) => return 0,
            Err(actual) => current = actual,
        }
    }
    set_errno(EAGAIN);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm {
    if time.is_null() || result.is_null() {
        set_errno(EFAULT);
        return core::ptr::null_mut();
    }
    unsafe {
        result.write(Tm {
            seconds: 0,
            minutes: 0,
            hours: 0,
            month_day: 1,
            month: 0,
            year: 70,
            week_day: 4,
            year_day: 0,
            daylight: 0,
            offset: 0,
            zone: b"UTC\0".as_ptr().cast(),
        });
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_syscall(number: i64, arguments: *mut c_void) -> isize {
    match number {
        SYS_GETPID => 1,
        SYS_GETTID => unsafe { pthread_self() as isize },
        SYS_SCHED_YIELD => unsafe { pthread_yield() as isize },
        SYS_CLOCK_GETTIME => {
            let clock_id = unsafe { variadic::next(arguments, VA_INT) } as i32;
            let time = unsafe { variadic::next(arguments, VA_POINTER) as *mut Timespec };
            unsafe { clock_gettime(clock_id, time) as isize }
        }
        SYS_FUTEX => {
            let address = unsafe { variadic::next(arguments, VA_POINTER) as *const u32 };
            let operation =
                unsafe { variadic::next(arguments, VA_INT) } as i32 & !FUTEX_PRIVATE_FLAG;
            let expected = unsafe { variadic::next(arguments, VA_UINT) } as u32;
            if address.is_null() {
                set_errno(EFAULT);
                -1
            } else if operation == FUTEX_WAKE {
                if expected == 0 {
                    0
                } else {
                    1
                }
            } else if operation == FUTEX_WAIT && unsafe { read_volatile(address) } != expected {
                set_errno(EAGAIN);
                -1
            } else {
                0
            }
        }
        _ => {
            set_errno(ENOSYS);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn __xpg_strerror_r(error: i32, buffer: *mut c_char, length: usize) -> i32 {
    if buffer.is_null() || length == 0 {
        return ERANGE;
    }
    let text = strerror_text(error);
    let text_length = text.len() - 1;
    let copied = text_length.min(length - 1);
    for index in 0..copied {
        unsafe { buffer.add(index).write(text[index] as c_char) };
    }
    unsafe { buffer.add(copied).write(0) };
    if copied == text_length {
        0
    } else {
        ERANGE
    }
}

#[no_mangle]
pub unsafe extern "C" fn strerror_r(error: i32, buffer: *mut c_char, length: usize) -> i32 {
    unsafe { __xpg_strerror_r(error, buffer, length) }
}

fn strerror_text(error: i32) -> &'static [u8] {
    match error {
        0 => b"success\0",
        EINVAL => b"invalid argument\0",
        ENOMEM => b"out of memory\0",
        EAGAIN => b"try again\0",
        EBUSY => b"resource busy\0",
        EIO => b"I/O error\0",
        EBADF => b"bad file descriptor\0",
        ENOSYS => b"function not implemented\0",
        _ => b"unknown error\0",
    }
}

#[no_mangle]
pub extern "C" fn strerror(error: i32) -> *mut c_char {
    let text = strerror_text(error);
    text.as_ptr().cast_mut().cast()
}

#[no_mangle]
pub extern "C" fn exit(_status: i32) -> ! {
    unsafe {
        __cxa_finalize(core::ptr::null_mut());
        fflush(core::ptr::null_mut());
    }
    trap()
}

#[no_mangle]
pub extern "C" fn getenv(_name: *const c_char) -> *mut c_char {
    core::ptr::null_mut()
}

unsafe fn parse_unsigned(input: *const c_char, end: *mut *mut c_char, base: i32) -> u64 {
    let start = input;
    let mut cursor = input;
    while matches!(
        unsafe { read_volatile(cursor.cast::<u8>()) },
        b' ' | b'\t'..=b'\r'
    ) {
        cursor = unsafe { cursor.add(1) };
    }
    let negative = unsafe { read_volatile(cursor) } == b'-' as c_char;
    if negative || unsafe { read_volatile(cursor) } == b'+' as c_char {
        cursor = unsafe { cursor.add(1) };
    }
    let mut radix = base;
    if (radix == 0 || radix == 16)
        && unsafe { read_volatile(cursor.cast::<u8>()) } == b'0'
        && matches!(
            unsafe { read_volatile(cursor.add(1).cast::<u8>()) },
            b'x' | b'X'
        )
    {
        radix = 16;
        cursor = unsafe { cursor.add(2) };
    } else if radix == 0 {
        radix = if unsafe { read_volatile(cursor.cast::<u8>()) } == b'0' {
            8
        } else {
            10
        };
    }
    if !(2..=36).contains(&radix) {
        set_errno(EINVAL);
        if !end.is_null() {
            unsafe { end.write(start.cast_mut()) };
        }
        return 0;
    }
    let first_digit = cursor;
    let mut value = 0u64;
    let mut overflow = false;
    loop {
        let character = unsafe { read_volatile(cursor.cast::<u8>()) };
        let digit = match character {
            b'0'..=b'9' => (character - b'0') as i32,
            b'a'..=b'z' => (character - b'a' + 10) as i32,
            b'A'..=b'Z' => (character - b'A' + 10) as i32,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        if value > (u64::MAX - digit as u64) / radix as u64 {
            overflow = true;
            value = u64::MAX;
        } else if !overflow {
            value = value * radix as u64 + digit as u64;
        }
        cursor = unsafe { cursor.add(1) };
    }
    if cursor == first_digit {
        if !end.is_null() {
            unsafe { end.write(start.cast_mut()) };
        }
        return 0;
    }
    if !end.is_null() {
        unsafe { end.write(cursor.cast_mut()) };
    }
    if overflow {
        set_errno(ERANGE);
    }
    if negative {
        0u64.wrapping_sub(value)
    } else {
        value
    }
}

#[no_mangle]
pub unsafe extern "C" fn strtoull(input: *const c_char, end: *mut *mut c_char, base: i32) -> u64 {
    unsafe { parse_unsigned(input, end, base) }
}

#[no_mangle]
pub unsafe extern "C" fn strtoul(input: *const c_char, end: *mut *mut c_char, base: i32) -> usize {
    let value = unsafe { parse_unsigned(input, end, base) };
    if value > usize::MAX as u64 {
        set_errno(ERANGE);
        usize::MAX
    } else {
        value as usize
    }
}

#[no_mangle]
pub unsafe extern "C" fn __isoc23_strtoull(
    input: *const c_char,
    end: *mut *mut c_char,
    base: i32,
) -> u64 {
    unsafe { strtoull(input, end, base) }
}

#[no_mangle]
pub unsafe extern "C" fn __isoc23_strtoul(
    input: *const c_char,
    end: *mut *mut c_char,
    base: i32,
) -> usize {
    unsafe { strtoul(input, end, base) }
}

#[no_mangle]
pub unsafe extern "C" fn __isoc23_strtol(
    input: *const c_char,
    end: *mut *mut c_char,
    base: i32,
) -> isize {
    unsafe { parse_unsigned(input, end, base) as isize }
}

#[no_mangle]
pub unsafe extern "C" fn __isoc23_strtoll(
    input: *const c_char,
    end: *mut *mut c_char,
    base: i32,
) -> i64 {
    unsafe { parse_unsigned(input, end, base) as i64 }
}

#[no_mangle]
pub unsafe extern "C" fn strtod(input: *const c_char, end: *mut *mut c_char) -> f64 {
    let start = input;
    let mut cursor = input;
    while matches!(
        unsafe { read_volatile(cursor.cast::<u8>()) },
        b' ' | b'\t'..=b'\r'
    ) {
        cursor = unsafe { cursor.add(1) };
    }
    let negative = unsafe { read_volatile(cursor) } == b'-' as c_char;
    if negative || unsafe { read_volatile(cursor) } == b'+' as c_char {
        cursor = unsafe { cursor.add(1) };
    }
    let mut value = 0.0f64;
    let mut digits = 0usize;
    loop {
        let character = unsafe { read_volatile(cursor.cast::<u8>()) };
        if !character.is_ascii_digit() {
            break;
        }
        value = value * 10.0 + (character - b'0') as f64;
        digits += 1;
        cursor = unsafe { cursor.add(1) };
    }
    if unsafe { read_volatile(cursor.cast::<u8>()) } == b'.' {
        cursor = unsafe { cursor.add(1) };
        let mut scale = 0.1;
        loop {
            let character = unsafe { read_volatile(cursor.cast::<u8>()) };
            if !character.is_ascii_digit() {
                break;
            }
            value += (character - b'0') as f64 * scale;
            scale *= 0.1;
            digits += 1;
            cursor = unsafe { cursor.add(1) };
        }
    }
    if digits == 0 {
        if !end.is_null() {
            unsafe { end.write(start.cast_mut()) };
        }
        return 0.0;
    }
    let exponent_start = cursor;
    let exponent_character = unsafe { read_volatile(cursor.cast::<u8>()) };
    if exponent_character == b'e' || exponent_character == b'E' {
        cursor = unsafe { cursor.add(1) };
        let exponent_negative = unsafe { read_volatile(cursor) } == b'-' as c_char;
        if exponent_negative || unsafe { read_volatile(cursor) } == b'+' as c_char {
            cursor = unsafe { cursor.add(1) };
        }
        let mut exponent = 0i32;
        let mut exponent_digits = 0usize;
        loop {
            let character = unsafe { read_volatile(cursor.cast::<u8>()) };
            if !character.is_ascii_digit() || exponent >= 400 {
                break;
            }
            exponent = exponent * 10 + (character - b'0') as i32;
            exponent_digits += 1;
            cursor = unsafe { cursor.add(1) };
        }
        if exponent_digits == 0 {
            cursor = exponent_start;
        } else {
            for _ in 0..exponent {
                value = if exponent_negative {
                    value / 10.0
                } else {
                    value * 10.0
                };
            }
        }
    }
    if !end.is_null() {
        unsafe { end.write(cursor.cast_mut()) };
    }
    if negative {
        -value
    } else {
        value
    }
}

#[no_mangle]
pub unsafe extern "C" fn strtof(input: *const c_char, end: *mut *mut c_char) -> f32 {
    unsafe { strtod(input, end) as f32 }
}

static CTYPE_B_STATE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static mut CTYPE_B_TABLE: [u16; 384] = [0; 384];
static mut CTYPE_B_POINTER: *const u16 = core::ptr::null();

#[no_mangle]
pub extern "C" fn __ctype_b_loc() -> *mut *const u16 {
    use core::sync::atomic::Ordering;
    if CTYPE_B_STATE
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        unsafe {
            let table = core::ptr::addr_of_mut!(CTYPE_B_TABLE).cast::<u16>();
            for value in 0..256usize {
                let mut flags = 0u16;
                if value < 32 || value == 127 {
                    flags |= 0x0002;
                }
                if value == b' ' as usize || (b'\t' as usize..=b'\r' as usize).contains(&value) {
                    flags |= 0x2001;
                }
                if (b'0' as usize..=b'9' as usize).contains(&value) {
                    flags |= 0x0808;
                }
                if (b'a' as usize..=b'z' as usize).contains(&value)
                    || (b'A' as usize..=b'Z' as usize).contains(&value)
                {
                    flags |= 0x0408;
                }
                if (b'a' as usize..=b'z' as usize).contains(&value) {
                    flags |= 0x0200;
                }
                if (b'A' as usize..=b'Z' as usize).contains(&value) {
                    flags |= 0x0100;
                }
                if (b'0' as usize..=b'9' as usize).contains(&value)
                    || (b'a' as usize..=b'f' as usize).contains(&value)
                    || (b'A' as usize..=b'F' as usize).contains(&value)
                {
                    flags |= 0x1000;
                }
                if (32..127).contains(&value) {
                    flags |= 0x4000;
                }
                if (33..127).contains(&value) {
                    flags |= 0x8000;
                }
                if (33..127).contains(&value)
                    && !(b'0' as usize..=b'9' as usize).contains(&value)
                    && !(b'a' as usize..=b'z' as usize).contains(&value)
                    && !(b'A' as usize..=b'Z' as usize).contains(&value)
                {
                    flags |= 0x0004;
                }
                table.add(value + 128).write(flags);
            }
            CTYPE_B_POINTER = table.add(128);
        }
        CTYPE_B_STATE.store(2, Ordering::Release);
    } else {
        while CTYPE_B_STATE.load(Ordering::Acquire) != 2 {
            core::hint::spin_loop();
        }
    }
    core::ptr::addr_of_mut!(CTYPE_B_POINTER)
}

#[no_mangle]
pub extern "C" fn setpriority(_which: i32, _who: i32, _priority: i32) -> i32 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn __open64_2(path: *const c_char, flags: i32) -> i32 {
    if flags & 0o100 != 0 {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { super::filesystem::kz_rust_open(path, flags, 0) }
}

#[no_mangle]
pub unsafe extern "C" fn kz_rust_mremap(
    old_address: *mut c_void,
    old_length: usize,
    new_length: usize,
    flags: i32,
) -> *mut c_void {
    const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;
    const PROT_READ_WRITE: i32 = 3;
    const MAP_PRIVATE_ANONYMOUS: i32 = 0x22;
    const MREMAP_MAYMOVE: i32 = 1;
    if old_address.is_null() || old_length == 0 || new_length == 0 || flags & !MREMAP_MAYMOVE != 0 {
        set_errno(EINVAL);
        return MAP_FAILED;
    }
    if old_length == new_length {
        return old_address;
    }
    let replacement = unsafe {
        mmap(
            core::ptr::null_mut(),
            new_length,
            PROT_READ_WRITE,
            MAP_PRIVATE_ANONYMOUS,
            -1,
            0,
        )
    };
    if replacement == MAP_FAILED {
        return MAP_FAILED;
    }
    let copy_length = old_length.min(new_length);
    for index in 0..copy_length {
        unsafe {
            write_volatile(
                replacement.cast::<u8>().add(index),
                read_volatile(old_address.cast::<u8>().add(index)),
            );
        }
    }
    if unsafe { munmap(old_address, old_length) } != 0 {
        unsafe { munmap(replacement, new_length) };
        return MAP_FAILED;
    }
    replacement
}

#[no_mangle]
pub unsafe extern "C" fn gettimeofday(value: *mut c_void, _timezone: *mut c_void) -> i32 {
    if value.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let milliseconds = unsafe { kz_host_wall_time_ms() };
    unsafe {
        value.cast::<Timeval>().write(Timeval {
            seconds: (milliseconds / 1000) as c_long,
            microseconds: (milliseconds % 1000) as c_long * 1000,
        });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn time(result: *mut i64) -> i64 {
    let seconds = unsafe { kz_host_wall_time_ms() } / 1000;
    if !result.is_null() {
        unsafe { result.write(seconds) };
    }
    seconds
}

#[no_mangle]
pub unsafe extern "C" fn sigaction(
    _signal_number: i32,
    _action: *const c_void,
    _old_action: *mut c_void,
) -> i32 {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigaltstack(_stack: *const c_void, _old_stack: *mut c_void) -> i32 {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub extern "C" fn feclearexcept(_exceptions: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn feraiseexcept(_exceptions: i32) -> i32 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn uname(output: *mut c_void) -> i32 {
    if output.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let name = output.cast::<Utsname>();
    unsafe {
        name.write(Utsname {
            sysname: fixed_field(b"kz"),
            nodename: fixed_field(b"localhost"),
            release: fixed_field(b"0.1"),
            version: fixed_field(b"freestanding"),
            machine: fixed_field(b"x86_64"),
            domainname: fixed_field(b"local"),
        });
    }
    0
}

fn fixed_field<const SIZE: usize>(value: &[u8]) -> [u8; SIZE] {
    let mut field = [0; SIZE];
    let length = value.len().min(SIZE.saturating_sub(1));
    field[..length].copy_from_slice(&value[..length]);
    field
}

#[cold]
fn trap() -> ! {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("brk #0", options(noreturn));
    }
}
