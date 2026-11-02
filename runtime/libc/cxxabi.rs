/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;

use crate::sync::IrqSpinLock;

const MAX_EXIT_HANDLERS: usize = 256;

type Destructor = unsafe extern "C" fn(*mut c_void);

#[derive(Clone, Copy)]
struct ExitHandler {
    destructor: Option<Destructor>,
    object: *mut c_void,
    dso: *mut c_void,
    active: bool,
}

unsafe impl Send for ExitHandler {}

impl ExitHandler {
    const EMPTY: Self = Self {
        destructor: None,
        object: core::ptr::null_mut(),
        dso: core::ptr::null_mut(),
        active: false,
    };
}

struct ExitHandlers {
    handlers: [ExitHandler; MAX_EXIT_HANDLERS],
    length: usize,
}

impl ExitHandlers {
    const fn new() -> Self {
        Self {
            handlers: [ExitHandler::EMPTY; MAX_EXIT_HANDLERS],
            length: 0,
        }
    }
}

static EXIT_HANDLERS: IrqSpinLock<ExitHandlers> = IrqSpinLock::new(ExitHandlers::new());

#[no_mangle]
pub extern "C" fn __cxa_atexit(
    destructor: Option<Destructor>,
    object: *mut c_void,
    dso: *mut c_void,
) -> i32 {
    let Some(destructor) = destructor else {
        return -1;
    };
    let mut state = EXIT_HANDLERS.lock();
    if state.length == MAX_EXIT_HANDLERS {
        return -1;
    }
    let index = state.length;
    state.handlers[index] = ExitHandler {
        destructor: Some(destructor),
        object,
        dso,
        active: true,
    };
    state.length += 1;
    0
}

#[no_mangle]
pub extern "C" fn __cxa_finalize(dso: *mut c_void) {
    loop {
        let handler = {
            let mut state = EXIT_HANDLERS.lock();
            let mut found = None;
            for index in (0..state.length).rev() {
                let entry = &mut state.handlers[index];
                if entry.active && (dso.is_null() || entry.dso == dso) {
                    entry.active = false;
                    found = Some(*entry);
                    break;
                }
            }
            found
        };
        let Some(handler) = handler else {
            break;
        };
        if let Some(destructor) = handler.destructor {
            unsafe { destructor(handler.object) };
        }
    }
}

#[no_mangle]
pub extern "C" fn _Unwind_GetRegionStart(_context: *mut c_void) -> usize {
    0
}

#[no_mangle]
pub extern "C" fn _Unwind_GetLanguageSpecificData(_context: *mut c_void) -> *mut c_void {
    core::ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn _Unwind_SetIP(_context: *mut c_void, _instruction_pointer: usize) {}

#[no_mangle]
pub extern "C" fn _Unwind_RaiseException(_exception: *mut c_void) -> i32 {
    3
}

#[no_mangle]
pub extern "C" fn _Unwind_Resume(_exception: *mut c_void) -> ! {
    trap()
}

#[no_mangle]
pub extern "C" fn _Unwind_DeleteException(_exception: *mut c_void) {}

#[no_mangle]
pub extern "C" fn _Unwind_Backtrace(
    _trace: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32>,
    _argument: *mut c_void,
) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn _Unwind_GetIP(_context: *mut c_void) -> usize {
    0
}

#[no_mangle]
pub extern "C" fn _Unwind_SetGR(
    _context: *mut c_void,
    _register_number: i32,
    _value: usize,
) {
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