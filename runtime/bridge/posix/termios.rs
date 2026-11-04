use core::ffi::{c_int, c_uint, c_void};
use super::errno::{EINVAL, ENOSYS};

#[repr(C)]
pub struct Termios {
    input_flags: c_uint,
    output_flags: c_uint,
    control_flags: c_uint,
    local_flags: c_uint,
    control_characters: [u8; 32],
    input_speed: c_uint,
    output_speed: c_uint,
}

#[no_mangle]
pub unsafe extern "C" fn cfgetispeed(attributes: *const Termios) -> c_uint {
    if attributes.is_null() {
        return 0;
    }
    unsafe { (*attributes).input_speed }
}

#[no_mangle]
pub unsafe extern "C" fn cfgetospeed(attributes: *const Termios) -> c_uint {
    if attributes.is_null() {
        return 0;
    }
    unsafe { (*attributes).output_speed }
}

#[no_mangle]
pub unsafe extern "C" fn cfsetispeed(attributes: *mut Termios, speed: c_uint) -> c_int {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe { (*attributes).input_speed = speed };
    0
}

#[no_mangle]
pub unsafe extern "C" fn cfsetospeed(attributes: *mut Termios, speed: c_uint) -> c_int {
    if attributes.is_null() {
        return EINVAL;
    }
    unsafe { (*attributes).output_speed = speed };
    0
}

// FIXME: Kernel virtio-console exposes byte I/O only; no TTY line discipline or termios API exists.
fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn tcdrain(_file_descriptor: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcflow(_file_descriptor: c_int, _action: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcflush(_file_descriptor: c_int, _queue_selector: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcgetattr(_file_descriptor: c_int, _attributes: *mut Termios) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcgetsid(_file_descriptor: c_int) -> i32 {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcgetwinsize(_file_descriptor: c_int, _size: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcsendbreak(_file_descriptor: c_int, _duration: c_int) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcsetattr(
    _file_descriptor: c_int,
    _actions: c_int,
    _attributes: *const Termios,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub unsafe extern "C" fn tcsetwinsize(_file_descriptor: c_int, _size: *const c_void) -> c_int {
    unavailable()
}