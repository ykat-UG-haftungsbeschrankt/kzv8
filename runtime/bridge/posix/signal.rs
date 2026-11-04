use core::ffi::{c_char, c_int, c_void};
use core::ptr::{read_volatile, write_volatile};
use super::errno::{EFAULT, EINVAL, ENOSYS};

const SIGNAL_SET_BYTES: usize = 128;

#[repr(C)]
#[derive(Clone, Copy)]
pub union Sigval {
    integer: c_int,
    pointer: *mut c_void,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn signal_bit(signal_number: c_int) -> Option<(usize, u8)> {
    if signal_number <= 0 || signal_number as usize > SIGNAL_SET_BYTES * 8 {
        None
    } else {
        let bit = signal_number as usize - 1;
        Some((bit / 8, 1 << (bit % 8)))
    }
}

#[no_mangle]
pub unsafe extern "C" fn sigaddset(set: *mut u8, signal_number: c_int) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Some((byte_index, bit)) = signal_bit(signal_number) else {
        set_errno(EINVAL);
        return -1;
    };
    unsafe {
        let byte = read_volatile(set.add(byte_index));
        write_volatile(set.add(byte_index), byte | bit);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigdelset(set: *mut u8, signal_number: c_int) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Some((byte_index, bit)) = signal_bit(signal_number) else {
        set_errno(EINVAL);
        return -1;
    };
    unsafe {
        let byte = read_volatile(set.add(byte_index));
        write_volatile(set.add(byte_index), byte & !bit);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn sigismember(set: *const u8, signal_number: c_int) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let Some((byte_index, bit)) = signal_bit(signal_number) else {
        set_errno(EINVAL);
        return -1;
    };
    ((unsafe { read_volatile(set.add(byte_index)) } & bit) != 0) as c_int
}

fn signal_name(signal_number: c_int) -> Option<&'static [u8]> {
    Some(match signal_number {
        1 => b"SIGHUP\0",
        2 => b"SIGINT\0",
        3 => b"SIGQUIT\0",
        4 => b"SIGILL\0",
        5 => b"SIGTRAP\0",
        6 => b"SIGABRT\0",
        7 => b"SIGBUS\0",
        8 => b"SIGFPE\0",
        9 => b"SIGKILL\0",
        10 => b"SIGUSR1\0",
        11 => b"SIGSEGV\0",
        12 => b"SIGUSR2\0",
        13 => b"SIGPIPE\0",
        14 => b"SIGALRM\0",
        15 => b"SIGTERM\0",
        17 => b"SIGCHLD\0",
        18 => b"SIGCONT\0",
        19 => b"SIGSTOP\0",
        20 => b"SIGTSTP\0",
        21 => b"SIGTTIN\0",
        22 => b"SIGTTOU\0",
        23 => b"SIGURG\0",
        24 => b"SIGXCPU\0",
        25 => b"SIGXFSZ\0",
        26 => b"SIGVTALRM\0",
        27 => b"SIGPROF\0",
        28 => b"SIGWINCH\0",
        29 => b"SIGPOLL\0",
        31 => b"SIGSYS\0",
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn sig2str(signal_number: c_int, output: *mut c_char) -> c_int {
    let Some(name) = signal_name(signal_number) else {
        return EINVAL;
    };
    if output.is_null() {
        return EFAULT;
    }
    for (index, byte) in name.iter().copied().enumerate() {
        unsafe { output.add(index).write(byte as c_char) };
    }
    0
}

unsafe fn string_equals(input: *const c_char, expected: &[u8]) -> bool {
    for (index, byte) in expected.iter().copied().enumerate() {
        if unsafe { read_volatile(input.cast::<u8>().add(index)) } != byte {
            return false;
        }
    }
    true
}

#[no_mangle]
pub unsafe extern "C" fn str2sig(input: *const c_char, signal_number: *mut c_int) -> c_int {
    if input.is_null() || signal_number.is_null() {
        return EFAULT;
    }
    for value in 1..=31 {
        if let Some(name) = signal_name(value) {
            if unsafe { string_equals(input, name) } {
                unsafe { signal_number.write(value) };
                return 0;
            }
        }
    }
    EINVAL
}

// FIXME: The kernel has no signal delivery, pending-signal queue, or process signal table.
#[no_mangle]
pub unsafe extern "C" fn kill(_process_id: i32, _signal_number: c_int) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn raise(_signal_number: c_int) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigpending(_set: *mut c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigprocmask(
    _how: c_int,
    _set: *const c_void,
    _old_set: *mut c_void,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigqueue(
    _process_id: i32,
    _signal_number: c_int,
    _value: Sigval,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigsuspend(_set: *const c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigtimedwait(
    _set: *const c_void,
    _information: *mut c_void,
    _timeout: *const c_void,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sigwait(_set: *const c_void, _signal_number: *mut c_int) -> c_int {
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn sigwaitinfo(_set: *const c_void, _information: *mut c_void) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn psignal(_signal_number: c_int, _message: *const c_char) {}

#[no_mangle]
pub unsafe extern "C" fn psiginfo(_information: *const c_void, _message: *const c_char) {}

#[no_mangle]
pub unsafe extern "C" fn signal(
    _signal_number: c_int,
    _handler: *mut c_void,
) -> *mut c_void {
    set_errno(ENOSYS);
    usize::MAX as *mut c_void
}