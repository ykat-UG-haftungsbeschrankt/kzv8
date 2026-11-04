use core::ffi::{c_char, c_void};
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};

use super::errno::EILSEQ;

const LOW_SURROGATE_PENDING: u32 = 0x8000_0000;
const HIGH_SURROGATE_PENDING: u32 = 0x4000_0000;
const SURROGATE_VALUE_MASK: u32 = 0xffff;

static MBR_INTERNAL_STATE: AtomicU32 = AtomicU32::new(0);
static C16_INTERNAL_STATE: AtomicU32 = AtomicU32::new(0);

pub(crate) unsafe fn state_is_initial(state: *const c_void) -> bool {
    if state.is_null() {
        MBR_INTERNAL_STATE.load(Ordering::Acquire) == 0
            && C16_INTERNAL_STATE.load(Ordering::Acquire) == 0
    } else {
        unsafe { read_volatile(state.cast::<u32>()) == 0 }
    }
}

unsafe extern "C" {
    fn mbrtowc(output: *mut i32, input: *const c_char, size: usize, state: *mut c_void)
        -> usize;
    fn wcrtomb(output: *mut c_char, character: i32, state: *mut c_void) -> usize;
    fn __errno_location() -> *mut i32;
}

fn set_errno(error: i32) {
    unsafe { __errno_location().write(error) };
}

unsafe fn read_state(state: *mut c_void, internal: &AtomicU32) -> u32 {
    if state.is_null() {
        internal.load(Ordering::Acquire)
    } else {
        unsafe { read_volatile(state.cast::<u32>()) }
    }
}

unsafe fn write_state(state: *mut c_void, internal: &AtomicU32, value: u32) {
    if state.is_null() {
        internal.store(value, Ordering::Release);
    } else {
        unsafe { write_volatile(state.cast::<u32>(), value) };
    }
}

fn invalid_sequence() -> usize {
    set_errno(EILSEQ);
    usize::MAX
}

#[no_mangle]
pub unsafe extern "C" fn mbrtoc16(
    output: *mut u16,
    input: *const c_char,
    size: usize,
    state: *mut c_void,
) -> usize {
    let current_state = unsafe { read_state(state, &MBR_INTERNAL_STATE) };
    if current_state & LOW_SURROGATE_PENDING != 0 {
        let low_surrogate = (current_state & SURROGATE_VALUE_MASK) as u16;
        unsafe { write_state(state, &MBR_INTERNAL_STATE, 0) };
        if !output.is_null() {
            unsafe { output.write(low_surrogate) };
        }
        return usize::MAX - 2;
    }

    let mut wide_character = 0i32;
    let result = unsafe { mbrtowc(&mut wide_character, input, size, state) };
    if result == usize::MAX {
        return invalid_sequence();
    }
    if result == usize::MAX - 1 {
        return result;
    }

    let character = wide_character as u32;
    if character <= 0xffff {
        if !output.is_null() {
            unsafe { output.write(character as u16) };
        }
        return result;
    }
    if character > 0x10ffff {
        return invalid_sequence();
    }

    let scalar = character - 0x1_0000;
    let high_surrogate = 0xd800 | (scalar >> 10);
    let low_surrogate = 0xdc00 | (scalar & 0x3ff);
    unsafe {
        write_state(
            state,
            &MBR_INTERNAL_STATE,
            LOW_SURROGATE_PENDING | low_surrogate,
        );
    }
    if !output.is_null() {
        unsafe { output.write(high_surrogate as u16) };
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn mbrtoc32(
    output: *mut u32,
    input: *const c_char,
    size: usize,
    state: *mut c_void,
) -> usize {
    let mut wide_character = 0i32;
    let result = unsafe { mbrtowc(&mut wide_character, input, size, state) };
    if result == usize::MAX {
        return invalid_sequence();
    }
    if result != usize::MAX - 1 && !output.is_null() {
        unsafe { output.write(wide_character as u32) };
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn c16rtomb(
    output: *mut c_char,
    character: u16,
    state: *mut c_void,
) -> usize {
    if output.is_null() {
        unsafe { write_state(state, &C16_INTERNAL_STATE, 0) };
        return unsafe { wcrtomb(core::ptr::null_mut(), 0, state) };
    }

    let current_state = unsafe { read_state(state, &C16_INTERNAL_STATE) };
    if (0xd800..=0xdbff).contains(&character) {
        if current_state & HIGH_SURROGATE_PENDING != 0 {
            unsafe { write_state(state, &C16_INTERNAL_STATE, 0) };
            return invalid_sequence();
        }
        unsafe {
            write_state(
                state,
                &C16_INTERNAL_STATE,
                HIGH_SURROGATE_PENDING | character as u32,
            );
        }
        return 0;
    }

    let scalar = if (0xdc00..=0xdfff).contains(&character) {
        if current_state & HIGH_SURROGATE_PENDING == 0 {
            unsafe { write_state(state, &C16_INTERNAL_STATE, 0) };
            return invalid_sequence();
        }
        let high = current_state & SURROGATE_VALUE_MASK;
        unsafe { write_state(state, &C16_INTERNAL_STATE, 0) };
        0x1_0000 + ((high - 0xd800) << 10) + (character as u32 - 0xdc00)
    } else {
        if current_state & HIGH_SURROGATE_PENDING != 0 {
            unsafe { write_state(state, &C16_INTERNAL_STATE, 0) };
            return invalid_sequence();
        }
        character as u32
    };

    let result = unsafe { wcrtomb(output, scalar as i32, state) };
    if result == usize::MAX {
        invalid_sequence()
    } else {
        result
    }
}

#[no_mangle]
pub unsafe extern "C" fn c32rtomb(
    output: *mut c_char,
    character: u32,
    state: *mut c_void,
) -> usize {
    let scalar = if output.is_null() { 0 } else { character };
    let result = unsafe { wcrtomb(output, scalar as i32, state) };
    if result == usize::MAX {
        invalid_sequence()
    } else {
        result
    }
}