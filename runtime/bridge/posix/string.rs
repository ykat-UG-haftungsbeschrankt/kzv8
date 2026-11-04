use core::ffi::{c_char, c_int, c_void};
use core::ptr::{copy_nonoverlapping, read_volatile, write_volatile};

unsafe extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn strcmp(left: *const c_char, right: *const c_char) -> c_int;
    fn strerror(error_number: c_int) -> *mut c_char;
}

unsafe fn length(text: *const u8) -> usize {
    let mut count = 0;
    while unsafe { read_volatile(text.add(count)) } != 0 {
        count += 1;
    }
    count
}

unsafe fn contains(set: *const u8, value: u8) -> bool {
    let mut index = 0;
    loop {
        let current = unsafe { read_volatile(set.add(index)) };
        if current == 0 {
            return false;
        }
        if current == value {
            return true;
        }
        index += 1;
    }
}


#[no_mangle]
pub unsafe extern "C" fn stpcpy(destination: *mut c_char, source: *const c_char) -> *mut c_char {
    let count = unsafe { length(source.cast()) };
    unsafe {
        copy_nonoverlapping(source.cast::<u8>(), destination.cast::<u8>(), count);
        destination.add(count).write(0);
    }
    unsafe { destination.add(count) }
}

#[no_mangle]
pub unsafe extern "C" fn stpncpy(
    destination: *mut c_char,
    source: *const c_char,
    count: usize,
) -> *mut c_char {
    let mut index = 0;
    while index < count {
        let value = unsafe { read_volatile(source.cast::<u8>().add(index)) };
        unsafe { write_volatile(destination.cast::<u8>().add(index), value) };
        index += 1;
        if value == 0 {
            break;
        }
    }
    let result = if index < count && unsafe { read_volatile(destination.cast::<u8>().add(index - 1)) } == 0 {
        unsafe { destination.add(index - 1) }
    } else {
        unsafe { destination.add(count) }
    };
    while index < count {
        unsafe { write_volatile(destination.cast::<u8>().add(index), 0) };
        index += 1;
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn strcat(destination: *mut c_char, source: *const c_char) -> *mut c_char {
    let destination_length = unsafe { length(destination.cast()) };
    let source_length = unsafe { length(source.cast()) };
    unsafe {
        copy_nonoverlapping(
            source.cast::<u8>(),
            destination.cast::<u8>().add(destination_length),
            source_length + 1,
        );
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn strcpy(destination: *mut c_char, source: *const c_char) -> *mut c_char {
    unsafe { stpcpy(destination, source) };
    destination
}

#[no_mangle]
pub unsafe extern "C" fn strcspn(text: *const c_char, reject: *const c_char) -> usize {
    let mut count = 0;
    loop {
        let value = unsafe { read_volatile(text.cast::<u8>().add(count)) };
        if value == 0 || unsafe { contains(reject.cast(), value) } {
            return count;
        }
        count += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strspn(text: *const c_char, accept: *const c_char) -> usize {
    let mut count = 0;
    loop {
        let value = unsafe { read_volatile(text.cast::<u8>().add(count)) };
        if value == 0 || !unsafe { contains(accept.cast(), value) } {
            return count;
        }
        count += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strpbrk(text: *const c_char, accept: *const c_char) -> *mut c_char {
    let mut index = 0;
    loop {
        let value = unsafe { read_volatile(text.cast::<u8>().add(index)) };
        if value == 0 {
            return core::ptr::null_mut();
        }
        if unsafe { contains(accept.cast(), value) } {
            return unsafe { text.add(index).cast_mut() };
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strrchr(text: *const c_char, character: c_int) -> *mut c_char {
    let target = character as u8;
    let mut last = core::ptr::null_mut();
    let mut index = 0;
    loop {
        let value = unsafe { read_volatile(text.cast::<u8>().add(index)) };
        if value == target {
            last = unsafe { text.add(index).cast_mut() };
        }
        if value == 0 {
            return last;
        }
        index += 1;
    }
}

#[no_mangle]
pub unsafe extern "C" fn strndup(text: *const c_char, maximum_length: usize) -> *mut c_char {
    let mut count = 0;
    while count < maximum_length && unsafe { read_volatile(text.cast::<u8>().add(count)) } != 0 {
        count += 1;
    }
    let Some(allocation_size) = count.checked_add(1) else {
        return core::ptr::null_mut();
    };
    let output = unsafe { malloc(allocation_size) }.cast::<u8>();
    if output.is_null() {
        return core::ptr::null_mut();
    }
    unsafe {
        copy_nonoverlapping(text.cast::<u8>(), output, count);
        output.add(count).write(0);
    }
    output.cast()
}

#[no_mangle]
pub unsafe extern "C" fn strdup(text: *const c_char) -> *mut c_char {
    unsafe { strndup(text, usize::MAX) }
}

#[no_mangle]
pub unsafe extern "C" fn strnlen(text: *const c_char, maximum_length: usize) -> usize {
    let mut count = 0;
    while count < maximum_length && unsafe { read_volatile(text.cast::<u8>().add(count)) } != 0 {
        count += 1;
    }
    count
}

#[no_mangle]
pub unsafe extern "C" fn strncat(
    destination: *mut c_char,
    source: *const c_char,
    maximum_count: usize,
) -> *mut c_char {
    let destination_length = unsafe { length(destination.cast()) };
    let source_length = unsafe { strnlen(source, maximum_count) };
    unsafe {
        copy_nonoverlapping(
            source.cast::<u8>(),
            destination.cast::<u8>().add(destination_length),
            source_length,
        );
        destination.cast::<u8>().add(destination_length + source_length).write(0);
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn strtok_r(
    text: *mut c_char,
    delimiters: *const c_char,
    state: *mut *mut c_char,
) -> *mut c_char {
    if state.is_null() || delimiters.is_null() {
        return core::ptr::null_mut();
    }
    let mut cursor = if text.is_null() { unsafe { state.read() } } else { text };
    if cursor.is_null() {
        return core::ptr::null_mut();
    }
    while unsafe { read_volatile(cursor.cast::<u8>()) } != 0
        && unsafe { contains(delimiters.cast(), read_volatile(cursor.cast::<u8>())) }
    {
        cursor = unsafe { cursor.add(1) };
    }
    if unsafe { read_volatile(cursor.cast::<u8>()) } == 0 {
        unsafe { state.write(core::ptr::null_mut()) };
        return core::ptr::null_mut();
    }
    let token = cursor;
    loop {
        let value = unsafe { read_volatile(cursor.cast::<u8>()) };
        if value == 0 {
            unsafe { state.write(core::ptr::null_mut()) };
            return token;
        }
        if unsafe { contains(delimiters.cast(), value) } {
            unsafe {
                cursor.write(0);
                state.write(cursor.add(1));
            }
            return token;
        }
        cursor = unsafe { cursor.add(1) };
    }
}

static mut STRTOK_STATE: *mut c_char = core::ptr::null_mut();

#[no_mangle]
pub unsafe extern "C" fn strtok(text: *mut c_char, delimiters: *const c_char) -> *mut c_char {
    unsafe { strtok_r(text, delimiters, core::ptr::addr_of_mut!(STRTOK_STATE)) }
}

#[no_mangle]
pub unsafe extern "C" fn strcoll(left: *const c_char, right: *const c_char) -> c_int {
    unsafe { strcmp(left, right) }
}

#[no_mangle]
pub unsafe extern "C" fn strcoll_l(
    left: *const c_char,
    right: *const c_char,
    _locale: *mut c_void,
) -> c_int {
    unsafe { strcmp(left, right) }
}

#[no_mangle]
pub unsafe extern "C" fn strxfrm(
    destination: *mut c_char,
    source: *const c_char,
    destination_size: usize,
) -> usize {
    let source_length = unsafe { length(source.cast()) };
    if !destination.is_null() && destination_size != 0 {
        let copied = source_length.min(destination_size - 1);
        for index in 0..copied {
            unsafe {
                write_volatile(
                    destination.cast::<u8>().add(index),
                    read_volatile(source.cast::<u8>().add(index)),
                );
            }
        }
        unsafe { destination.cast::<u8>().add(copied).write(0) };
    }
    source_length
}

#[no_mangle]
pub unsafe extern "C" fn strxfrm_l(
    destination: *mut c_char,
    source: *const c_char,
    destination_size: usize,
    _locale: *mut c_void,
) -> usize {
    unsafe { strxfrm(destination, source, destination_size) }
}

#[no_mangle]
pub unsafe extern "C" fn strlcpy(
    destination: *mut c_char,
    source: *const c_char,
    destination_size: usize,
) -> usize {
    let source_length = unsafe { length(source.cast()) };
    if !destination.is_null() && destination_size != 0 {
        let copied = source_length.min(destination_size - 1);
        unsafe {
            copy_nonoverlapping(source.cast::<u8>(), destination.cast::<u8>(), copied);
            destination.cast::<u8>().add(copied).write(0);
        }
    }
    source_length
}

#[no_mangle]
pub unsafe extern "C" fn strlcat(
    destination: *mut c_char,
    source: *const c_char,
    destination_size: usize,
) -> usize {
    let source_length = unsafe { length(source.cast()) };
    let destination_length = unsafe { strnlen(destination, destination_size) };
    if destination_length == destination_size {
        return destination_size.saturating_add(source_length);
    }
    let available = destination_size - destination_length;
    let copied = source_length.min(available.saturating_sub(1));
    unsafe {
        copy_nonoverlapping(
            source.cast::<u8>(),
            destination.cast::<u8>().add(destination_length),
            copied,
        );
        destination.cast::<u8>().add(destination_length + copied).write(0);
    }
    destination_length + source_length
}

#[no_mangle]
pub unsafe extern "C" fn strerror_l(error_number: c_int, _locale: *mut c_void) -> *mut c_char {
    unsafe { strerror(error_number) }
}

#[no_mangle]
pub unsafe extern "C" fn strsignal(signal_number: c_int) -> *mut c_char {
    let message: &'static [u8] = match signal_number {
        1 => b"Hangup\0",
        2 => b"Interrupt\0",
        3 => b"Quit\0",
        6 => b"Aborted\0",
        9 => b"Killed\0",
        11 => b"Segmentation fault\0",
        13 => b"Broken pipe\0",
        14 => b"Alarm clock\0",
        15 => b"Terminated\0",
        _ => b"Unknown signal\0",
    };
    message.as_ptr().cast_mut().cast()
}

#[no_mangle]
pub unsafe extern "C" fn memmem(
    haystack: *const c_void,
    haystack_size: usize,
    needle: *const c_void,
    needle_size: usize,
) -> *mut c_void {
    if needle_size == 0 {
        return haystack.cast_mut();
    }
    if haystack.is_null() || needle.is_null() || needle_size > haystack_size {
        return core::ptr::null_mut();
    }
    let haystack = haystack.cast::<u8>();
    let needle = needle.cast::<u8>();
    for start in 0..=haystack_size - needle_size {
        let mut matched = true;
        for index in 0..needle_size {
            if unsafe { read_volatile(haystack.add(start + index)) }
                != unsafe { read_volatile(needle.add(index)) }
            {
                matched = false;
                break;
            }
        }
        if matched {
            return unsafe { haystack.add(start).cast_mut().cast() };
        }
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn memccpy(
    destination: *mut c_void,
    source: *const c_void,
    character: c_int,
    count: usize,
) -> *mut c_void {
    let destination = destination.cast::<u8>();
    let source = source.cast::<u8>();
    for index in 0..count {
        let value = unsafe { read_volatile(source.add(index)) };
        unsafe { write_volatile(destination.add(index), value) };
        if value == character as u8 {
            return unsafe { destination.add(index + 1).cast() };
        }
    }
    core::ptr::null_mut()
}