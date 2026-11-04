use core::ffi::c_int;
use super::errno::{EINVAL, ENOSYS};

const SCHED_OTHER: c_int = 0;
const SCHED_FIFO: c_int = 1;
const SCHED_RR: c_int = 2;
const SCHED_SPORADIC: c_int = 3;

#[repr(C)]
pub struct SchedParam {
    priority: c_int,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn valid_process(process_id: i32) -> bool {
    process_id == 0 || process_id == 1
}

#[no_mangle]
pub extern "C" fn sched_get_priority_max(policy: c_int) -> c_int {
    if policy == SCHED_OTHER {
        0
    } else if matches!(policy, SCHED_FIFO | SCHED_RR | SCHED_SPORADIC) {
        set_errno(ENOSYS);
        -1
    } else {
        set_errno(EINVAL);
        -1
    }
}

#[no_mangle]
pub extern "C" fn sched_get_priority_min(policy: c_int) -> c_int {
    sched_get_priority_max(policy)
}

#[no_mangle]
pub unsafe extern "C" fn sched_getparam(
    process_id: i32,
    parameters: *mut SchedParam,
) -> c_int {
    if !valid_process(process_id) {
        set_errno(EINVAL);
        return -1;
    }
    if parameters.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { (*parameters).priority = 0 };
    0
}

#[no_mangle]
pub extern "C" fn sched_getscheduler(process_id: i32) -> c_int {
    if valid_process(process_id) {
        SCHED_OTHER
    } else {
        set_errno(EINVAL);
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn sched_rr_get_interval(
    _process_id: i32,
    _interval: *mut core::ffi::c_void,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sched_setparam(
    _process_id: i32,
    _parameters: *const SchedParam,
) -> c_int {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn sched_setscheduler(
    _process_id: i32,
    _policy: c_int,
    _parameters: *const SchedParam,
) -> c_int {
    set_errno(ENOSYS);
    -1
}