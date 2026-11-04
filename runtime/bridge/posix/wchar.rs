use core::ffi::{c_char, c_int, c_void};
use core::ptr::{copy_nonoverlapping, read_volatile, write_volatile};
use super::errno::ENOSYS;

const WEOF: u32 = u32::MAX;

unsafe extern "C" {
    static mut stdin: *mut c_void;
    static mut stdout: *mut c_void;
    fn fgetc(stream: *mut c_void) -> c_int;
    fn fputwc(character: i32, stream: *mut c_void) -> u32;
    fn malloc(size: usize) -> *mut c_void;
    fn mbrtowc(output: *mut i32, input: *const c_char, size: usize, state: *mut c_void) -> usize;
    fn wcsnrtombs(
        output: *mut c_char,
        input: *mut *const i32,
        character_count: usize,
        byte_count: usize,
        state: *mut c_void,
    ) -> usize;
}

unsafe fn wide_length(text: *const i32) -> usize {
    let mut length = 0;
    while unsafe { read_volatile(text.add(length)) } != 0 {
        length += 1;
    }
    length
}

unsafe fn compare_wide(left: *const i32, right: *const i32, limit: Option<usize>) -> c_int {
    let mut index = 0;
    loop {
        if limit.is_some_and(|count| index == count) {
            return 0;
        }
        let left_value = unsafe { read_volatile(left.add(index)) };
        let right_value = unsafe { read_volatile(right.add(index)) };
        if left_value != right_value {
            return left_value.cmp(&right_value) as c_int;
        }
        if left_value == 0 {
            return 0;
        }
        index += 1;
    }
}

#[no_mangle]
pub extern "C" fn btowc(character: c_int) -> u32 {
    if character == -1 || !(0..=0x7f).contains(&character) {
        WEOF
    } else {
        character as u32
    }
}

#[no_mangle]
pub unsafe extern "C" fn fgetwc(stream: *mut c_void) -> u32 {
    let first = unsafe { fgetc(stream) };
    if first < 0 {
        return WEOF;
    }
    let first_byte = first as u8;
    let expected = if first_byte < 0x80 {
        1
    } else if first_byte & 0xe0 == 0xc0 {
        2
    } else if first_byte & 0xf0 == 0xe0 {
        3
    } else if first_byte & 0xf8 == 0xf0 {
        4
    } else {
        return WEOF;
    };
    let mut encoded = [0u8; 4];
    encoded[0] = first_byte;
    for index in 1..expected {
        let next = unsafe { fgetc(stream) };
        if next < 0 {
            return WEOF;
        }
        encoded[index] = next as u8;
    }
    let mut character = 0;
    let converted = unsafe {
        mbrtowc(
            &mut character,
            encoded.as_ptr().cast(),
            expected,
            core::ptr::null_mut(),
        )
    };
    if converted == usize::MAX || converted == usize::MAX - 1 {
        WEOF
    } else {
        character as u32
    }
}

#[no_mangle]
pub unsafe extern "C" fn fgetws(
    output: *mut i32,
    size: c_int,
    stream: *mut c_void,
) -> *mut i32 {
    if output.is_null() || stream.is_null() || size <= 0 {
        return core::ptr::null_mut();
    }
    let mut count = 0;
    while count + 1 < size as usize {
        let character = unsafe { fgetwc(stream) };
        if character == WEOF {
            if count == 0 {
                return core::ptr::null_mut();
            }
            break;
        }
        unsafe { output.add(count).write(character as i32) };
        count += 1;
        if character == b'\n' as u32 {
            break;
        }
    }
    unsafe { output.add(count).write(0) };
    output
}

#[no_mangle]
pub unsafe extern "C" fn fputws(text: *const i32, stream: *mut c_void) -> c_int {
    if text.is_null() || stream.is_null() {
        return -1;
    }
    let mut index = 0;
    loop {
        let character = unsafe { read_volatile(text.add(index)) };
        if character == 0 {
            return 0;
        }
        if unsafe { fputwc(character, stream) } == WEOF {
            return -1;
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn fwide(_stream: *mut c_void, _mode: c_int) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn getwchar() -> u32 {
    let stream = unsafe { core::ptr::addr_of!(stdin).read_volatile() };
    unsafe { fgetwc(stream) }
}

#[no_mangle]
pub unsafe extern "C" fn putwc(character: i32, stream: *mut c_void) -> u32 {
    unsafe { fputwc(character, stream) }
}

#[no_mangle]
pub unsafe extern "C" fn putwchar(character: i32) -> u32 {
    let stream = unsafe { core::ptr::addr_of!(stdout).read_volatile() };
    unsafe { fputwc(character, stream) }
}

#[no_mangle]
pub unsafe extern "C" fn mbsinit(state: *const c_void) -> c_int {
    if unsafe { super::uchar::state_is_initial(state) } {
        1
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcscasecmp(left: *const i32, right: *const i32) -> c_int {
    let mut index = 0;
    loop {
        let left_value = unsafe { read_volatile(left.add(index)) };
        let right_value = unsafe { read_volatile(right.add(index)) };
        let folded_left = if (0..=255).contains(&left_value) { (left_value as u8).to_ascii_lowercase() as i32 } else { left_value };
        let folded_right = if (0..=255).contains(&right_value) { (right_value as u8).to_ascii_lowercase() as i32 } else { right_value };
        if folded_left != folded_right {
            return folded_left.cmp(&folded_right) as c_int;
        }
        if left_value == 0 {
            return 0;
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcscasecmp_l(left: *const i32, right: *const i32, _locale: *mut c_void) -> c_int {
    unsafe { wcscasecmp(left, right) }
}

#[no_mangle]
pub unsafe extern "C" fn wcscat(destination: *mut i32, source: *const i32) -> *mut i32 {
    let destination_length = unsafe { wide_length(destination) };
    let source_length = unsafe { wide_length(source) };
    unsafe {
        copy_nonoverlapping(source, destination.add(destination_length), source_length + 1);
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn wcschr(text: *const i32, character: i32) -> *mut i32 {
    let mut index = 0;
    loop {
        let value = unsafe { read_volatile(text.add(index)) };
        if value == character {
            return unsafe { text.add(index).cast_mut() };
        }
        if value == 0 {
            return core::ptr::null_mut();
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcscmp(left: *const i32, right: *const i32) -> c_int {
    unsafe { compare_wide(left, right, None) }
}

#[no_mangle]
pub unsafe extern "C" fn wcscoll(left: *const i32, right: *const i32) -> c_int {
    unsafe { wcscmp(left, right) }
}

#[no_mangle]
pub unsafe extern "C" fn wcscoll_l(left: *const i32, right: *const i32, _locale: *mut c_void) -> c_int {
    unsafe { wcscmp(left, right) }
}

#[no_mangle]
pub unsafe extern "C" fn wcscpy(destination: *mut i32, source: *const i32) -> *mut i32 {
    let count = unsafe { wide_length(source) } + 1;
    unsafe { copy_nonoverlapping(source, destination, count) };
    destination
}

#[no_mangle]
pub unsafe extern "C" fn wcpcpy(destination: *mut i32, source: *const i32) -> *mut i32 {
    unsafe { wcscpy(destination, source) };
    unsafe { destination.add(wide_length(source)) }
}

#[no_mangle]
pub unsafe extern "C" fn wcpncpy(destination: *mut i32, source: *const i32, count: usize) -> *mut i32 {
    let mut index = 0;
    while index < count {
        let value = unsafe { read_volatile(source.add(index)) };
        unsafe { write_volatile(destination.add(index), value) };
        index += 1;
        if value == 0 {
            break;
        }
    }
    let result = if index > 0 && unsafe { read_volatile(destination.add(index - 1)) } == 0 {
        unsafe { destination.add(index - 1) }
    } else {
        unsafe { destination.add(count) }
    };
    while index < count {
        unsafe { write_volatile(destination.add(index), 0) };
        index += 1;
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn wcscspn(text: *const i32, reject: *const i32) -> usize {
    let mut count = 0;
    loop {
        let value = unsafe { read_volatile(text.add(count)) };
        if value == 0 || unsafe { wide_contains(reject, value) } {
            return count;
        }
        count += 1;
    }
}

unsafe fn wide_contains(set: *const i32, character: i32) -> bool {
    let mut index = 0;
    loop {
        let value = unsafe { read_volatile(set.add(index)) };
        if value == 0 { return false; }
        if value == character { return true; }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcsdup(text: *const i32) -> *mut i32 {
    let count = unsafe { wide_length(text) } + 1;
    let Some(bytes) = count.checked_mul(core::mem::size_of::<i32>()) else {
        return core::ptr::null_mut();
    };
    let output = unsafe { malloc(bytes) }.cast::<i32>();
    if output.is_null() { return output; }
    unsafe { copy_nonoverlapping(text, output, count) };
    output
}

#[no_mangle]
pub unsafe extern "C" fn wcslcat(destination: *mut i32, source: *const i32, size: usize) -> usize {
    let source_length = unsafe { wide_length(source) };
    let destination_length = unsafe { wcsnlen(destination, size) };
    if destination_length == size { return size.saturating_add(source_length); }
    let copied = source_length.min(size.saturating_sub(destination_length + 1));
    for index in 0..copied { unsafe { write_volatile(destination.add(destination_length + index), read_volatile(source.add(index))) }; }
    unsafe { write_volatile(destination.add(destination_length + copied), 0) };
    destination_length + source_length
}

#[no_mangle]
pub unsafe extern "C" fn wcslcpy(destination: *mut i32, source: *const i32, size: usize) -> usize {
    let source_length = unsafe { wide_length(source) };
    if size != 0 {
        let copied = source_length.min(size - 1);
        for index in 0..copied { unsafe { write_volatile(destination.add(index), read_volatile(source.add(index))) }; }
        unsafe { write_volatile(destination.add(copied), 0) };
    }
    source_length
}

#[no_mangle]
pub unsafe extern "C" fn wcslen(text: *const i32) -> usize {
    unsafe { wide_length(text) }
}

#[no_mangle]
pub unsafe extern "C" fn wcsncasecmp(left: *const i32, right: *const i32, count: usize) -> c_int {
    let mut index = 0;
    while index < count {
        let a = unsafe { read_volatile(left.add(index)) };
        let b = unsafe { read_volatile(right.add(index)) };
        let a = if (0..=255).contains(&a) { (a as u8).to_ascii_lowercase() as i32 } else { a };
        let b = if (0..=255).contains(&b) { (b as u8).to_ascii_lowercase() as i32 } else { b };
        if a != b { return a.cmp(&b) as c_int; }
        if a == 0 { return 0; }
        index += 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn wcsncasecmp_l(left: *const i32, right: *const i32, count: usize, _locale: *mut c_void) -> c_int {
    unsafe { wcsncasecmp(left, right, count) }
}

#[no_mangle]
pub unsafe extern "C" fn wcsncat(destination: *mut i32, source: *const i32, count: usize) -> *mut i32 {
    let destination_length = unsafe { wide_length(destination) };
    let copied = unsafe { wcsnlen(source, count) };
    for index in 0..copied { unsafe { write_volatile(destination.add(destination_length + index), read_volatile(source.add(index))) }; }
    unsafe { write_volatile(destination.add(destination_length + copied), 0) };
    destination
}

#[no_mangle]
pub unsafe extern "C" fn wcsncmp(left: *const i32, right: *const i32, count: usize) -> c_int {
    unsafe { compare_wide(left, right, Some(count)) }
}

#[no_mangle]
pub unsafe extern "C" fn wcsncpy(destination: *mut i32, source: *const i32, count: usize) -> *mut i32 {
    let mut index = 0;
    while index < count {
        let value = unsafe { read_volatile(source.add(index)) };
        unsafe { write_volatile(destination.add(index), value) };
        index += 1;
        if value == 0 { break; }
    }
    while index < count { unsafe { write_volatile(destination.add(index), 0) }; index += 1; }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn wcsnlen(text: *const i32, maximum: usize) -> usize {
    let mut count = 0;
    while count < maximum && unsafe { read_volatile(text.add(count)) } != 0 { count += 1; }
    count
}

#[no_mangle]
pub unsafe extern "C" fn wcspbrk(text: *const i32, accept: *const i32) -> *mut i32 {
    let mut index = 0;
    loop {
        let character = unsafe { read_volatile(text.add(index)) };
        if character == 0 { return core::ptr::null_mut(); }
        if unsafe { wide_contains(accept, character) } { return unsafe { text.add(index).cast_mut() }; }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcsrchr(text: *const i32, character: i32) -> *mut i32 {
    let mut last = core::ptr::null_mut();
    let mut index = 0;
    loop {
        let value = unsafe { read_volatile(text.add(index)) };
        if value == character { last = unsafe { text.add(index).cast_mut() }; }
        if value == 0 { return last; }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcsrtombs(output: *mut c_char, input: *mut *const i32, size: usize, state: *mut c_void) -> usize {
    unsafe { wcsnrtombs(output, input, usize::MAX, size, state) }
}

#[no_mangle]
pub unsafe extern "C" fn wcsspn(text: *const i32, accept: *const i32) -> usize {
    let mut count = 0;
    while unsafe { read_volatile(text.add(count)) } != 0 && unsafe { wide_contains(accept, read_volatile(text.add(count))) } { count += 1; }
    count
}

#[no_mangle]
pub unsafe extern "C" fn wcsstr(haystack: *const i32, needle: *const i32) -> *mut i32 {
    let needle_length = unsafe { wide_length(needle) };
    if needle_length == 0 { return haystack.cast_mut(); }
    let haystack_length = unsafe { wide_length(haystack) };
    if needle_length > haystack_length { return core::ptr::null_mut(); }
    for start in 0..=haystack_length - needle_length {
        let mut matched = true;
        for index in 0..needle_length {
            if unsafe { read_volatile(haystack.add(start + index)) } != unsafe { read_volatile(needle.add(index)) } { matched = false; break; }
        }
        if matched { return unsafe { haystack.add(start).cast_mut() }; }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn wcstok(
    text: *mut i32,
    delimiters: *const i32,
    state: *mut *mut i32,
) -> *mut i32 {
    if state.is_null() {
        return core::ptr::null_mut();
    }
    let mut cursor = if text.is_null() {
        unsafe { read_volatile(state) }
    } else {
        text
    };
    if cursor.is_null() {
        return core::ptr::null_mut();
    }

    while unsafe { read_volatile(cursor) } != 0
        && unsafe { wide_contains(delimiters, read_volatile(cursor)) }
    {
        cursor = unsafe { cursor.add(1) };
    }
    if unsafe { read_volatile(cursor) } == 0 {
        unsafe { write_volatile(state, cursor) };
        return core::ptr::null_mut();
    }

    let token = cursor;
    loop {
        let character = unsafe { read_volatile(cursor) };
        if character == 0 {
            unsafe { write_volatile(state, cursor) };
            return token;
        }
        if unsafe { wide_contains(delimiters, character) } {
            unsafe {
                write_volatile(cursor, 0);
                write_volatile(state, cursor.add(1));
            }
            return token;
        }
        cursor = unsafe { cursor.add(1) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn wcsxfrm(destination: *mut i32, source: *const i32, count: usize) -> usize {
    let source_length = unsafe { wide_length(source) };
    if count != 0 {
        let copied = source_length.min(count - 1);
        for index in 0..copied { unsafe { write_volatile(destination.add(index), read_volatile(source.add(index))) }; }
        unsafe { write_volatile(destination.add(copied), 0) };
    }
    source_length
}

#[no_mangle]
pub unsafe extern "C" fn wcsxfrm_l(destination: *mut i32, source: *const i32, count: usize, _locale: *mut c_void) -> usize {
    unsafe { wcsxfrm(destination, source, count) }
}

#[no_mangle]
pub unsafe extern "C" fn wctob(character: u32) -> c_int {
    if character <= 0x7f { character as c_int } else { -1 }
}

#[no_mangle]
pub unsafe extern "C" fn wcwidth(character: i32) -> c_int {
    if character == 0 { 0 } else if character < 0x20 || (0x7f..0xa0).contains(&character) { -1 } else if character <= 0x7e { 1 } else { -1 }
}

#[no_mangle]
pub unsafe extern "C" fn wcswidth(text: *const i32, count: usize) -> c_int {
    let mut width = 0;
    for index in 0..count {
        let character = unsafe { read_volatile(text.add(index)) };
        if character == 0 { break; }
        let character_width = unsafe { wcwidth(character) };
        if character_width < 0 { return -1; }
        width += character_width;
    }
    width
}

#[no_mangle]
pub unsafe extern "C" fn wmemchr(memory: *const i32, character: i32, count: usize) -> *mut i32 {
    for index in 0..count { if unsafe { read_volatile(memory.add(index)) } == character { return unsafe { memory.add(index).cast_mut() }; } }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn wmemcmp(left: *const i32, right: *const i32, count: usize) -> c_int {
    for index in 0..count {
        let a = unsafe { read_volatile(left.add(index)) };
        let b = unsafe { read_volatile(right.add(index)) };
        if a != b { return a.cmp(&b) as c_int; }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn wmemcpy(destination: *mut i32, source: *const i32, count: usize) -> *mut i32 {
    unsafe { copy_nonoverlapping(source, destination, count) };
    destination
}

#[no_mangle]
pub unsafe extern "C" fn wmemmove(destination: *mut i32, source: *const i32, count: usize) -> *mut i32 {
    unsafe { core::ptr::copy(source, destination, count) };
    destination
}

#[no_mangle]
pub unsafe extern "C" fn wmemset(memory: *mut i32, value: i32, count: usize) -> *mut i32 {
    for index in 0..count { unsafe { write_volatile(memory.add(index), value) }; }
    memory
}

// FIXME: Locale-aware wide numeric conversion and wide formatted streams need libc locale/format support.
fn unsupported() -> usize {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(ENOSYS) };
    0
}

#[no_mangle]
pub unsafe extern "C" fn open_wmemstream(
    _buffer: *mut *mut i32,
    _size: *mut usize,
) -> *mut c_void {
    unsupported();
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn wcsftime(_output: *mut i32, _size: usize, _format: *const i32, _time: *const c_void) -> usize { unsupported() }
#[no_mangle]
pub unsafe extern "C" fn wcstod(_text: *const i32, _end: *mut *mut i32) -> f64 { unsupported(); 0.0 }
#[no_mangle]
pub unsafe extern "C" fn wcstof(_text: *const i32, _end: *mut *mut i32) -> f32 { unsupported(); 0.0 }
#[no_mangle]
pub unsafe extern "C" fn wcstol(_text: *const i32, _end: *mut *mut i32, _base: c_int) -> isize { unsupported() as isize }
#[no_mangle]
pub unsafe extern "C" fn wcstoll(_text: *const i32, _end: *mut *mut i32, _base: c_int) -> i64 { unsupported() as i64 }
#[no_mangle]
pub unsafe extern "C" fn wcstoul(_text: *const i32, _end: *mut *mut i32, _base: c_int) -> usize { unsupported() }
#[no_mangle]
pub unsafe extern "C" fn wcstoull(_text: *const i32, _end: *mut *mut i32, _base: c_int) -> u64 { unsupported() as u64 }