/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::mem::size_of;
use core::ptr;
use core::slice;

use crate::kernel::runtime::runtime_libc::{network, stdlib};

const IF_NAMESIZE: usize = 16;

#[repr(C)]
pub struct IfNameIndex {
    interface_index: c_uint,
    interface_name: *mut c_char,
}

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub unsafe extern "C" fn if_freenameindex(interfaces: *mut IfNameIndex) {
    if interfaces.is_null() {
        return;
    }
    let mut index = 0;
    loop {
        let entry = unsafe { interfaces.add(index).read() };
        if entry.interface_index == 0 {
            break;
        }
        unsafe { stdlib::free(entry.interface_name.cast::<c_void>()) };
        index += 1;
    }
    unsafe { stdlib::free(interfaces.cast::<c_void>()) };
}

#[no_mangle]
pub unsafe extern "C" fn if_indextoname(
    interface_index: c_uint,
    name: *mut c_char,
) -> *mut c_char {
    if name.is_null() {
        set_errno(-(crate::kernel::error::Error::Fault as c_int));
        return ptr::null_mut();
    }
    let interfaces = match network::interface_list() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            set_errno(-(error as c_int));
            return ptr::null_mut();
        }
    };
    let Some((_, interface_name)) = interfaces
        .iter()
        .find(|(index, _)| *index == interface_index)
    else {
        set_errno(-(crate::kernel::error::Error::NoDev as c_int));
        return ptr::null_mut();
    };
    unsafe {
        ptr::copy_nonoverlapping(interface_name.as_ptr(), name.cast::<u8>(), IF_NAMESIZE);
    }
    name
}

#[no_mangle]
pub unsafe extern "C" fn if_nameindex() -> *mut IfNameIndex {
    let interfaces = match network::interface_list() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            set_errno(-(error as c_int));
            return ptr::null_mut();
        }
    };
    let entry_count = interfaces.len();
    let Some(array_size) = entry_count
        .checked_add(1)
        .and_then(|count| count.checked_mul(size_of::<IfNameIndex>()))
    else {
        set_errno(-(crate::kernel::error::Error::Overflow as c_int));
        return ptr::null_mut();
    };
    let output = unsafe { stdlib::malloc(array_size) }.cast::<IfNameIndex>();
    if output.is_null() {
        set_errno(-(crate::kernel::error::Error::NoMem as c_int));
        return ptr::null_mut();
    }
    for (index, (interface_index, interface_name)) in interfaces.iter().enumerate() {
        let name_length = interface_name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(IF_NAMESIZE);
        let name_pointer = unsafe { stdlib::malloc(name_length + 1) }.cast::<c_char>();
        if name_pointer.is_null() {
            let partial = unsafe { slice::from_raw_parts(output, index) };
            for entry in partial {
                unsafe { stdlib::free(entry.interface_name.cast::<c_void>()) };
            }
            unsafe { stdlib::free(output.cast::<c_void>()) };
            set_errno(-(crate::kernel::error::Error::NoMem as c_int));
            return ptr::null_mut();
        }
        unsafe {
            ptr::copy_nonoverlapping(interface_name.as_ptr(), name_pointer.cast::<u8>(), name_length);
            name_pointer.add(name_length).write(0);
            output.add(index).write(IfNameIndex {
                interface_index: *interface_index,
                interface_name: name_pointer,
            });
        }
    }
    unsafe {
        output.add(entry_count).write(IfNameIndex {
            interface_index: 0,
            interface_name: ptr::null_mut(),
        });
    }
    output
}

#[no_mangle]
pub unsafe extern "C" fn if_nametoindex(name: *const c_char) -> c_uint {
    if name.is_null() {
        set_errno(-(crate::kernel::error::Error::Fault as c_int));
        return 0;
    }
    let name = unsafe { core::ffi::CStr::from_ptr(name) }.to_bytes();
    if name.len() >= IF_NAMESIZE {
        set_errno(-(crate::kernel::error::Error::NameTooLong as c_int));
        return 0;
    }
    let interfaces = match network::interface_list() {
        Ok(interfaces) => interfaces,
        Err(error) => {
            set_errno(-(error as c_int));
            return 0;
        }
    };
    interfaces
        .iter()
        .find_map(|(index, interface_name)| {
            let length = interface_name
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(IF_NAMESIZE);
            (interface_name[..length] == *name).then_some(*index)
        })
        .unwrap_or_else(|| {
            set_errno(-(crate::kernel::error::Error::NoDev as c_int));
            0
        })
}