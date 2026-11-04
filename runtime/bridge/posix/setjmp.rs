use core::ffi::c_int;
use super::errno::ENOSYS;

#[repr(C)]
pub struct JumpBuffer {
    opaque: [usize; 32],
}

#[repr(C)]
pub struct SignalJumpBuffer {
    opaque: [usize; 34],
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

// FIXME: The kernel exposes no architecture-specific non-local register/stack save API.
#[no_mangle]
pub unsafe extern "C" fn setjmp(_environment: *mut JumpBuffer) -> c_int {
    set_errno(ENOSYS);
    0
}

#[no_mangle]
pub unsafe extern "C" fn longjmp(_environment: *mut JumpBuffer, _value: c_int) -> ! {
    set_errno(ENOSYS);
    trap()
}

#[no_mangle]
pub unsafe extern "C" fn sigsetjmp(
    _environment: *mut SignalJumpBuffer,
    _save_signal_mask: c_int,
) -> c_int {
    set_errno(ENOSYS);
    0
}

#[no_mangle]
pub unsafe extern "C" fn siglongjmp(
    _environment: *mut SignalJumpBuffer,
    _value: c_int,
) -> ! {
    set_errno(ENOSYS);
    trap()
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