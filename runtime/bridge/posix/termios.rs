/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_int, c_uint, c_void};

use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::{as_errno, EBADF, EFAULT, EINVAL, ENOTTY};

const TCSANOW: c_int = 0;
const TCSADRAIN: c_int = 1;
const TCSAFLUSH: c_int = 2;
const TCIFLUSH: c_int = 0;
const TCOFLUSH: c_int = 1;
const TCIOFLUSH: c_int = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct WindowSize {
    rows: u16,
    columns: u16,
    x_pixels: u16,
    y_pixels: u16,
}

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

fn set_errno(error: c_int) {
    unsafe { crate::kernel::scheduler::thread::__errno_location().write(error) };
}

fn validate_terminal(file_descriptor: c_int) -> Result<(), c_int> {
    match crate::kernel::runtime::runtime_libc::filesystem::is_terminal_descriptor(
        file_descriptor,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ENOTTY),
        Err(EBADF)
            if crate::kernel::runtime::runtime_libc::network::has_socket_fd(file_descriptor)
                || crate::kernel::runtime::runtime_libc::network::has_packet_fd(file_descriptor) =>
        {
            Err(ENOTTY)
        }
        Err(error) => Err(error),
    }
}

fn kernel_result(result: Result<(), crate::kernel::error::Error>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn tcdrain(file_descriptor: c_int) -> c_int {
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    if crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor) {
        return 0;
    }
    kernel_result(crate::kernel::tty::drain_output())
}

#[no_mangle]
pub unsafe extern "C" fn tcflow(file_descriptor: c_int, action: c_int) -> c_int {
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    if crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor) {
        return match crate::kernel::runtime::runtime_libc::pty::flow(file_descriptor, action) {
            Ok(()) => 0,
            Err(error) => kernel_result(Err(error)),
        };
    }
    kernel_result(crate::kernel::tty::flow(action))
}

#[no_mangle]
pub unsafe extern "C" fn tcflush(file_descriptor: c_int, queue_selector: c_int) -> c_int {
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    let is_pty = crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor);
    match queue_selector {
        TCIFLUSH if is_pty => {
            return match crate::kernel::runtime::runtime_libc::pty::flush_input(file_descriptor) {
                Ok(()) => 0,
                Err(error) => kernel_result(Err(error)),
            };
        }
        TCIFLUSH => crate::kernel::tty::flush_input(),
        TCOFLUSH if is_pty => {
            return match crate::kernel::runtime::runtime_libc::pty::flush_output(file_descriptor) {
                Ok(()) => 0,
                Err(error) => kernel_result(Err(error)),
            };
        }
        TCOFLUSH => crate::kernel::tty::flush_output(),
        TCIOFLUSH => {
            if is_pty {
                if let Err(error) = crate::kernel::runtime::runtime_libc::pty::flush_input(file_descriptor) {
                    return kernel_result(Err(error));
                }
                if let Err(error) = crate::kernel::runtime::runtime_libc::pty::flush_output(file_descriptor) {
                    return kernel_result(Err(error));
                }
            } else {
                crate::kernel::tty::flush_input();
                crate::kernel::tty::flush_output();
            }
        }
        _ => {
            set_errno(EINVAL);
            return -1;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn tcgetattr(file_descriptor: c_int, attributes: *mut Termios) -> c_int {
    if attributes.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    let attributes_value = if crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor) {
        match crate::kernel::runtime::runtime_libc::pty::attributes(file_descriptor) {
            Ok(attributes) => attributes,
            Err(error) => return kernel_result(Err(error)),
        }
    } else {
        crate::kernel::tty::attributes()
    };
    unsafe {
        core::ptr::write_volatile(
            attributes,
            Termios {
                input_flags: attributes_value.input_flags,
                output_flags: attributes_value.output_flags,
                control_flags: attributes_value.control_flags,
                local_flags: attributes_value.local_flags,
                control_characters: attributes_value.control_characters,
                input_speed: attributes_value.input_speed,
                output_speed: attributes_value.output_speed,
            },
        );
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn tcgetsid(_file_descriptor: c_int) -> i32 {
    let terminal = crate::kernel::runtime::runtime_libc::filesystem::is_terminal_descriptor(
        _file_descriptor,
    );
    let terminal = match terminal {
        Err(EBADF)
            if crate::kernel::runtime::runtime_libc::network::has_socket_fd(_file_descriptor)
                || crate::kernel::runtime::runtime_libc::network::has_packet_fd(_file_descriptor) =>
        {
            Ok(false)
        }
        result => result,
    };
    match terminal {
        Ok(true) => {}
        Ok(false) => {
            set_errno(ENOTTY);
            return -1;
        }
        Err(error) => {
            set_errno(error);
            return -1;
        }
    }
    match ThreadManager::current_process()
        .and_then(|process| process.controlling_terminal_session())
    {
        Ok(session_id) => session_id,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn tcgetwinsize(file_descriptor: c_int, size: *mut c_void) -> c_int {
    if size.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    let window = if crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor) {
        match crate::kernel::runtime::runtime_libc::pty::window_size(file_descriptor) {
            Ok(window) => window,
            Err(error) => return kernel_result(Err(error)),
        }
    } else {
        crate::kernel::tty::window_size()
    };
    unsafe {
        core::ptr::write_volatile(
            size.cast::<WindowSize>(),
            WindowSize {
                rows: window.rows,
                columns: window.columns,
                x_pixels: window.x_pixels,
                y_pixels: window.y_pixels,
            },
        );
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn tcsendbreak(file_descriptor: c_int, duration: c_int) -> c_int {
    if duration < 0 {
        set_errno(EINVAL);
        return -1;
    }
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    kernel_result(crate::kernel::tty::drain_output())
}

#[no_mangle]
pub unsafe extern "C" fn tcsetattr(
    file_descriptor: c_int,
    actions: c_int,
    attributes: *const Termios,
) -> c_int {
    if attributes.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    if !matches!(actions, TCSANOW | TCSADRAIN | TCSAFLUSH) {
        set_errno(EINVAL);
        return -1;
    }
    let is_pty = crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor);
    if actions != TCSANOW && !is_pty {
        if let Err(error) = crate::kernel::tty::drain_output() {
            set_errno(as_errno(error));
            return -1;
        }
    }
    if actions == TCSAFLUSH {
        if is_pty {
            if let Err(error) = crate::kernel::runtime::runtime_libc::pty::flush_input(file_descriptor) {
                return kernel_result(Err(error));
            }
        } else {
            crate::kernel::tty::flush_input();
        }
    }
    let attributes = unsafe { core::ptr::read_volatile(attributes) };
    let attributes = crate::kernel::tty::TerminalAttributes {
            input_flags: attributes.input_flags,
            output_flags: attributes.output_flags,
            control_flags: attributes.control_flags,
            local_flags: attributes.local_flags,
            control_characters: attributes.control_characters,
            input_speed: attributes.input_speed,
            output_speed: attributes.output_speed,
        };
    if is_pty {
        kernel_result(crate::kernel::runtime::runtime_libc::pty::set_attributes(
            file_descriptor,
            attributes,
        ))
    } else {
        kernel_result(crate::kernel::tty::set_attributes(attributes))
    }
}

#[no_mangle]
pub unsafe extern "C" fn tcsetwinsize(file_descriptor: c_int, size: *const c_void) -> c_int {
    if size.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    if let Err(error) = validate_terminal(file_descriptor) {
        set_errno(error);
        return -1;
    }
    let size = unsafe { core::ptr::read_volatile(size.cast::<WindowSize>()) };
    crate::kernel::tty::set_window_size(crate::kernel::tty::WindowSize {
        rows: size.rows,
        columns: size.columns,
        x_pixels: size.x_pixels,
        y_pixels: size.y_pixels,
    });
    0
}