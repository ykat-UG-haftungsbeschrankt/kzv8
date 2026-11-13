/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::ffi::{c_char, c_int, c_long, c_void};
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};

use crate::kernel::dl::AddressSpace;
use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::error::Error as KernelError;
use crate::kernel::process::{Process, MAX_SUPPLEMENTARY_GROUPS};
use crate::kernel::scheduler::thread::ThreadManager;

use super::errno::{as_errno, EBADF, EFAULT, EINVAL, ENAMETOOLONG, ENOMEM, ENOSYS, ENOTTY, ERANGE};

const CS_PATH: c_int = 0;
const O_CLOEXEC: c_int = 0o2000000;
const O_NONBLOCK: c_int = 0o4000;
const POSIX_CLOSE_RESTART: c_int = 1;

#[repr(C)]
struct Timespec {
    seconds: c_long,
    nanoseconds: c_long,
}

#[repr(C)]
struct Utsname {
    system_name: [u8; 65],
    node_name: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domain_name: [u8; 65],
}

unsafe extern "C" {
    fn __errno_location() -> *mut c_int;
    fn close(file_descriptor: c_int) -> c_int;
    fn kz_rust_chdir(path: *const c_char) -> c_int;
    fn kz_rust_fchdir(file_descriptor: c_int) -> c_int;
    fn kz_rust_unlinkat(
        directory_descriptor: c_int,
        path: *const c_char,
        flags: c_int,
    ) -> c_int;
    fn kz_rust_chown(path: *const c_char, owner: u32, group: u32) -> c_int;
    fn kz_rust_fchown(file_descriptor: c_int, owner: u32, group: u32) -> c_int;
    fn kz_rust_chownat(
        directory_descriptor: c_int,
        path: *const c_char,
        owner: u32,
        group: u32,
        flags: c_int,
    ) -> c_int;
    fn kz_rust_lchown(path: *const c_char, owner: u32, group: u32) -> c_int;
    fn kz_rust_link(source_path: *const c_char, destination_path: *const c_char) -> c_int;
    fn kz_rust_linkat(
        source_directory: c_int,
        source_path: *const c_char,
        destination_directory: c_int,
        destination_path: *const c_char,
        flags: c_int,
    ) -> c_int;
    fn kz_rust_symlink(target: *const c_char, link_path: *const c_char) -> c_int;
    fn kz_rust_symlinkat(
        target: *const c_char,
        directory_descriptor: c_int,
        link_path: *const c_char,
    ) -> c_int;
    fn kz_rust_readlink(path: *const c_char, buffer: *mut c_void, size: usize) -> isize;
    fn kz_rust_readlinkat(
        directory_descriptor: c_int,
        path: *const c_char,
        buffer: *mut c_void,
        size: usize,
    ) -> isize;
    fn nanosleep(request: *const Timespec, remaining: *mut Timespec) -> c_int;
    fn uname(output: *mut c_void) -> c_int;
}

#[no_mangle]
pub static mut environ: *mut *mut c_char = core::ptr::null_mut();

#[no_mangle]
pub static mut optarg: *mut c_char = core::ptr::null_mut();

#[no_mangle]
pub static mut opterr: c_int = 1;

#[no_mangle]
pub static mut optind: c_int = 1;

#[no_mangle]
pub static mut optopt: c_int = 0;

static mut NEXT_OPTION: *const c_char = core::ptr::null();
static mut NEXT_OPTION_INDEX: c_int = 0;

fn set_errno(error: c_int) {
    unsafe { __errno_location().write(error) };
}

fn set_kernel_errno(error: KernelError) {
    if error == KernelError::Intr {
        let _ = crate::kernel::runtime::runtime_posix::signal::dispatch_pending_signals();
    }
    set_errno(as_errno(error));
}

fn current_process_error() -> ProcessError {
    match ThreadManager::current_process() {
        Ok(process) => ProcessError::Current(process),
        Err(error) => ProcessError::Failed(error),
    }
}

enum ProcessError {
    Current(alloc::sync::Arc<Process>),
    Failed(KernelError),
}

fn credential_query(select: impl FnOnce(&Process) -> u32) -> u32 {
    match current_process_error() {
        ProcessError::Current(process) => select(&process),
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            u32::MAX
        }
    }
}

fn credential_update(update: impl FnOnce(&Process) -> Result<(), KernelError>) -> c_int {
    match current_process_error() {
        ProcessError::Current(process) => match update(&process) {
            Ok(()) => 0,
            Err(error) => {
                set_kernel_errno(error);
                -1
            }
        },
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

fn process_arc_update(update: impl FnOnce(&alloc::sync::Arc<Process>) -> Result<(), KernelError>) -> c_int {
    match current_process_error() {
        ProcessError::Current(process) => match update(&process) {
            Ok(()) => 0,
            Err(error) => {
                set_kernel_errno(error);
                -1
            }
        },
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

fn process_query(query: impl FnOnce(&Process) -> Result<i32, KernelError>) -> i32 {
    match current_process_error() {
        ProcessError::Current(process) => match query(&process) {
            Ok(value) => value,
            Err(error) => {
                set_kernel_errno(error);
                -1
            }
        },
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

fn terminal_descriptor_status(file_descriptor: c_int) -> Result<bool, c_int> {
    if file_descriptor < 0 {
        return Err(EBADF);
    }
    match crate::kernel::runtime::runtime_libc::filesystem::is_terminal_descriptor(file_descriptor)
    {
        Ok(is_terminal) => Ok(is_terminal),
        Err(EBADF)
            if crate::kernel::runtime::runtime_libc::network::has_socket_fd(file_descriptor)
                || crate::kernel::runtime::runtime_libc::network::has_packet_fd(file_descriptor) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn fail_int() -> c_int {
    set_errno(ENOSYS);
    -1
}

fn fail_long() -> c_long {
    set_errno(ENOSYS);
    -1
}

fn fail_pointer<T>() -> *mut T {
    set_errno(ENOSYS);
    core::ptr::null_mut()
}

unsafe fn clear_next_option() {
    unsafe {
        core::ptr::addr_of_mut!(NEXT_OPTION).write_volatile(core::ptr::null());
        core::ptr::addr_of_mut!(NEXT_OPTION_INDEX).write_volatile(0);
    }
}

unsafe fn save_next_option(pointer: *const c_char, index: c_int) {
    unsafe {
        core::ptr::addr_of_mut!(NEXT_OPTION).write_volatile(pointer);
        core::ptr::addr_of_mut!(NEXT_OPTION_INDEX).write_volatile(index);
    }
}

unsafe fn option_byte(pointer: *const c_char, index: usize) -> u8 {
    unsafe { read_volatile(pointer.cast::<u8>().add(index)) }
}

#[no_mangle]
pub unsafe extern "C" fn confstr(name: c_int, buffer: *mut c_char, size: usize) -> usize {
    if name != CS_PATH {
        set_errno(EINVAL);
        return 0;
    }
    if buffer.is_null() && size != 0 {
        set_errno(EFAULT);
        return 0;
    }
    const PATH: &[u8] = b"/bin:/usr/bin\0";
    if !buffer.is_null() {
        for index in 0..size.min(PATH.len()) {
            unsafe { write_volatile(buffer.cast::<u8>().add(index), PATH[index]) };
        }
    }
    PATH.len()
}

// FIXME: The runtime has no process-specific current-directory or permission facility.
#[no_mangle]
pub unsafe extern "C" fn access(_path: *const c_char, _mode: c_int) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn faccessat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _mode: c_int,
    _flags: c_int,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn chdir(path: *const c_char) -> c_int {
    unsafe { kz_rust_chdir(path) }
}

#[no_mangle]
pub extern "C" fn fchdir(file_descriptor: c_int) -> c_int {
    unsafe { kz_rust_fchdir(file_descriptor) }
}

#[no_mangle]
pub unsafe extern "C" fn chown(
    path: *const c_char,
    owner: u32,
    group: u32,
) -> c_int {
    unsafe { kz_rust_chown(path, owner, group) }
}

#[no_mangle]
pub extern "C" fn fchown(file_descriptor: c_int, owner: u32, group: u32) -> c_int {
    unsafe { kz_rust_fchown(file_descriptor, owner, group) }
}

#[no_mangle]
pub unsafe extern "C" fn fchownat(
    directory_descriptor: c_int,
    path: *const c_char,
    owner: u32,
    group: u32,
    flags: c_int,
) -> c_int {
    if flags & !0x100 != 0 {
        set_errno(EINVAL);
        return -1;
    }
    unsafe { kz_rust_chownat(directory_descriptor, path, owner, group, flags) }
}

#[no_mangle]
pub unsafe extern "C" fn lchown(path: *const c_char, owner: u32, group: u32) -> c_int {
    unsafe { kz_rust_lchown(path, owner, group) }
}

fn duplicate_descriptor_to(
    file_descriptor: c_int,
    new_descriptor: c_int,
    close_on_exec: bool,
) -> Result<c_int, c_int> {
    if new_descriptor < 0 {
        return Err(EBADF);
    }
    if file_descriptor == new_descriptor {
        return Ok(new_descriptor);
    }
    crate::kernel::runtime::runtime_libc::posix::check_descriptor_limit(new_descriptor)
        .map_err(|error| -(error as c_int))?;
    let process_id = ThreadManager::current_process()
        .map_err(as_errno)?
        .pid();
    crate::kernel::runtime::runtime_libc::filesystem::duplicate_descriptor_to_process(
        process_id,
        file_descriptor,
        process_id,
        new_descriptor,
        close_on_exec,
    )
}

#[no_mangle]
pub extern "C" fn dup(file_descriptor: c_int) -> c_int {
    let result = if crate::kernel::runtime::runtime_libc::pty::is_fd(file_descriptor) {
        crate::kernel::runtime::runtime_libc::pty::duplicate(file_descriptor, 0, false)
            .map_err(|error| -(error as i32))
    } else if crate::kernel::runtime::runtime_libc::network::is_socket_fd(file_descriptor) {
        crate::kernel::runtime::runtime_libc::network::duplicate_socket(file_descriptor, 0, false)
            .map_err(|error| -(error as i32))
    } else if crate::kernel::runtime::runtime_libc::network::is_packet_fd(file_descriptor) {
        crate::kernel::runtime::runtime_libc::network::duplicate_packet_fd(file_descriptor, 0, false)
            .map_err(|error| -(error as i32))
    } else {
        crate::kernel::runtime::runtime_libc::filesystem::duplicate(file_descriptor, 3, false)
    };
    match result {
        Ok(descriptor) => descriptor,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn dup2(file_descriptor: c_int, new_descriptor: c_int) -> c_int {
    let result = duplicate_descriptor_to(file_descriptor, new_descriptor, false);
    match result {
        Ok(descriptor) => descriptor,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn dup3(
    file_descriptor: c_int,
    new_descriptor: c_int,
    flags: c_int,
) -> c_int {
    if flags & !O_CLOEXEC != 0 || file_descriptor == new_descriptor {
        set_errno(EINVAL);
        return -1;
    }
    let result = duplicate_descriptor_to(
        file_descriptor,
        new_descriptor,
        flags & O_CLOEXEC != 0,
    );
    match result {
        Ok(descriptor) => descriptor,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

fn create_pipe(file_descriptors: *mut c_int, flags: c_int) -> c_int {
    if file_descriptors.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    if !process
        .address_space()
        .user_range_accessible(file_descriptors as usize, 2 * size_of::<c_int>(), true)
    {
        set_errno(EFAULT);
        return -1;
    }
    let (read_descriptor, write_descriptor) =
        match crate::kernel::runtime::runtime_libc::filesystem::create_pipe(flags) {
            Ok(descriptors) => descriptors,
            Err(error) => {
                set_errno(error);
                return -1;
            }
        };
    let descriptors = [read_descriptor, write_descriptor];
    let bytes = unsafe {
        core::slice::from_raw_parts(descriptors.as_ptr().cast::<u8>(), size_of::<[c_int; 2]>())
    };
    if let Err(error) = process.address_space().write(file_descriptors as usize, bytes) {
        let _ = unsafe { close(read_descriptor) };
        let _ = unsafe { close(write_descriptor) };
        set_kernel_errno(error);
        return -1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn pipe(file_descriptors: *mut c_int) -> c_int {
    create_pipe(file_descriptors, 0)
}

#[no_mangle]
pub unsafe extern "C" fn pipe2(file_descriptors: *mut c_int, flags: c_int) -> c_int {
    if flags & !(O_NONBLOCK | O_CLOEXEC) != 0 {
        set_errno(EINVAL);
        return -1;
    }
    create_pipe(file_descriptors, flags)
}

#[no_mangle]
pub extern "C" fn posix_close(file_descriptor: c_int, flags: c_int) -> c_int {
    if flags & !POSIX_CLOSE_RESTART != 0 {
        return EINVAL;
    }
    if unsafe { close(file_descriptor) } == 0 {
        0
    } else {
        unsafe { __errno_location().read() }
    }
}

#[no_mangle]
pub extern "C" fn fork() -> i32 {
    fail_int()
}

#[no_mangle]
pub extern "C" fn _Fork() -> i32 {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn execv(path: *const c_char, arguments: *const *mut c_char) -> c_int {
    unsafe { crate::kernel::runtime::runtime_posix::exec::execve(path, arguments, environ) }
}

#[no_mangle]
pub unsafe extern "C" fn execve(
    path: *const c_char,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> c_int {
    unsafe { crate::kernel::runtime::runtime_posix::exec::execve(path, arguments, environment) }
}

#[no_mangle]
pub unsafe extern "C" fn execvp(file: *const c_char, arguments: *const *mut c_char) -> c_int {
    unsafe {
        crate::kernel::runtime::runtime_posix::exec::execvpe(file, arguments, environ, true)
    }
}

#[no_mangle]
pub unsafe extern "C" fn fexecve(
    file_descriptor: c_int,
    arguments: *const *mut c_char,
    environment: *const *mut c_char,
) -> c_int {
    unsafe {
        crate::kernel::runtime::runtime_posix::exec::fexecve(
            file_descriptor,
            arguments,
            environment,
        )
    }
}

#[no_mangle]
pub extern "C" fn fdatasync(file_descriptor: c_int) -> c_int {
    match crate::kernel::runtime::runtime_libc::filesystem::sync_descriptor(file_descriptor, true) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn fsync(file_descriptor: c_int) -> c_int {
    match crate::kernel::runtime::runtime_libc::filesystem::sync_descriptor(file_descriptor, false) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn ftruncate(file_descriptor: c_int, length: i64) -> c_int {
    if crate::kernel::runtime::runtime_libc::network::is_socket_fd(file_descriptor)
        || crate::kernel::runtime::runtime_libc::network::is_packet_fd(file_descriptor)
    {
        set_kernel_errno(KernelError::Spipe);
        return -1;
    }
    match crate::kernel::runtime::runtime_libc::filesystem::truncate_descriptor(
        file_descriptor,
        length,
    ) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn truncate(path: *const c_char, length: i64) -> c_int {
    match crate::kernel::runtime::runtime_libc::filesystem::truncate_path(path, length) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn getentropy(buffer: *mut c_void, size: usize) -> c_int {
    if buffer.is_null() && size != 0 {
        set_errno(EFAULT);
        return -1;
    }
    if size > 256 {
        set_kernel_errno(KernelError::Io);
        return -1;
    }
    if size == 0 {
        return 0;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let mut random = Vec::new();
    if random.try_reserve_exact(size).is_err() {
        set_errno(ENOMEM);
        return -1;
    }
    random.resize(size, 0);
    if let Err(error) = crate::random::fill(&mut random) {
        set_kernel_errno(error);
        return -1;
    }
    match process.address_space().write(buffer as usize, &random) {
        Ok(()) => 0,
        Err(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn gethostname(name: *mut c_char, size: usize) -> c_int {
    if name.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let mut system_name = Utsname {
        system_name: [0; 65],
        node_name: [0; 65],
        release: [0; 65],
        version: [0; 65],
        machine: [0; 65],
        domain_name: [0; 65],
    };
    if unsafe { uname((&mut system_name as *mut Utsname).cast()) } != 0 {
        return -1;
    }
    let host_length = system_name
        .node_name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(system_name.node_name.len());
    if size <= host_length {
        set_errno(ENAMETOOLONG);
        return -1;
    }
    for index in 0..=host_length {
        unsafe { write_volatile(name.cast::<u8>().add(index), system_name.node_name[index]) };
    }
    0
}

#[no_mangle]
pub extern "C" fn getegid() -> u32 {
    credential_query(|process| process.group_ids()[1])
}

#[no_mangle]
pub extern "C" fn geteuid() -> u32 {
    credential_query(|process| process.user_ids()[1])
}

#[no_mangle]
pub extern "C" fn getgid() -> u32 {
    credential_query(|process| process.group_ids()[0])
}

#[no_mangle]
pub extern "C" fn getuid() -> u32 {
    credential_query(|process| process.user_ids()[0])
}

#[no_mangle]
pub unsafe extern "C" fn getgroups(group_count: c_int, groups: *mut u32) -> c_int {
    if group_count < 0 {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process_error() {
        ProcessError::Current(process) => process,
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let supplementary_groups = match process.supplementary_group_ids() {
        Ok(groups) => groups,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    if group_count == 0 {
        return supplementary_groups.len() as c_int;
    }
    if groups.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    if (group_count as usize) < supplementary_groups.len() {
        set_errno(EINVAL);
        return -1;
    }
    for (index, group) in supplementary_groups.iter().copied().enumerate() {
        unsafe { write_volatile(groups.add(index), group) };
    }
    supplementary_groups.len() as c_int
}

#[no_mangle]
pub unsafe extern "C" fn getresgid(
    real_group: *mut u32,
    effective_group: *mut u32,
    saved_group: *mut u32,
) -> c_int {
    if real_group.is_null() || effective_group.is_null() || saved_group.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    match current_process_error() {
        ProcessError::Current(process) => {
            let [real, effective, saved] = process.group_ids();
            unsafe {
                write_volatile(real_group, real);
                write_volatile(effective_group, effective);
                write_volatile(saved_group, saved);
            }
            0
        }
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn getresuid(
    real_user: *mut u32,
    effective_user: *mut u32,
    saved_user: *mut u32,
) -> c_int {
    if real_user.is_null() || effective_user.is_null() || saved_user.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    match current_process_error() {
        ProcessError::Current(process) => {
            let [real, effective, saved] = process.user_ids();
            unsafe {
                write_volatile(real_user, real);
                write_volatile(effective_user, effective);
                write_volatile(saved_user, saved);
            }
            0
        }
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn setgroups(group_count: usize, groups: *const u32) -> c_int {
    if group_count > MAX_SUPPLEMENTARY_GROUPS {
        set_errno(EINVAL);
        return -1;
    }
    let process = match current_process_error() {
        ProcessError::Current(process) => process,
        ProcessError::Failed(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    if !process.effective_user_is_root() {
        set_errno(super::errno::EPERM);
        return -1;
    }
    if group_count != 0 && groups.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let mut group_ids = Vec::new();
    if group_ids.try_reserve_exact(group_count).is_err() {
        set_errno(ENOMEM);
        return -1;
    }
    for index in 0..group_count {
        group_ids.push(unsafe { read_volatile(groups.add(index)) });
    }
    credential_update(|process| process.set_supplementary_group_ids(&group_ids))
}

// FIXME: The runtime has no account database.
#[no_mangle]
pub unsafe extern "C" fn getlogin() -> *mut c_char {
    fail_pointer()
}

#[no_mangle]
pub unsafe extern "C" fn getlogin_r(_buffer: *mut c_char, _size: usize) -> c_int {
    ENOSYS
}

#[no_mangle]
pub extern "C" fn gethostid() -> c_long {
    fail_long()
}

#[no_mangle]
pub unsafe extern "C" fn getpgid(process: i32) -> i32 {
    process_query(|current| current.process_group_for(process))
}

#[no_mangle]
pub extern "C" fn getpgrp() -> i32 {
    process_query(|process| Ok(process.process_group_id()))
}

#[no_mangle]
pub extern "C" fn getppid() -> i32 {
    process_query(|process| Ok(process.parent_pid()))
}

#[no_mangle]
pub unsafe extern "C" fn getsid(process: i32) -> i32 {
    process_query(|current| current.session_for(process))
}

#[no_mangle]
pub extern "C" fn setegid(group: u32) -> c_int {
    credential_update(|process| process.set_effective_group(group))
}

#[no_mangle]
pub extern "C" fn seteuid(user: u32) -> c_int {
    credential_update(|process| process.set_effective_user(user))
}

#[no_mangle]
pub extern "C" fn setgid(group: u32) -> c_int {
    credential_update(|process| process.set_group_id(group))
}

#[no_mangle]
pub extern "C" fn setpgid(process: i32, group: i32) -> c_int {
    process_arc_update(|current| current.set_process_group(process, group))
}

#[no_mangle]
pub extern "C" fn setpgrp() -> c_int {
    process_arc_update(|current| current.set_process_group(0, 0))
}

#[no_mangle]
pub extern "C" fn setregid(real_group: u32, effective_group: u32) -> c_int {
    credential_update(|process| {
        process.set_real_and_effective_group(
            (real_group != u32::MAX).then_some(real_group),
            (effective_group != u32::MAX).then_some(effective_group),
        )
    })
}

#[no_mangle]
pub extern "C" fn setresgid(real_group: u32, effective_group: u32, saved_group: u32) -> c_int {
    credential_update(|process| {
        process.set_all_group_ids(
            (real_group != u32::MAX).then_some(real_group),
            (effective_group != u32::MAX).then_some(effective_group),
            (saved_group != u32::MAX).then_some(saved_group),
        )
    })
}

#[no_mangle]
pub extern "C" fn setresuid(real_user: u32, effective_user: u32, saved_user: u32) -> c_int {
    credential_update(|process| {
        process.set_all_user_ids(
            (real_user != u32::MAX).then_some(real_user),
            (effective_user != u32::MAX).then_some(effective_user),
            (saved_user != u32::MAX).then_some(saved_user),
        )
    })
}

#[no_mangle]
pub extern "C" fn setreuid(real_user: u32, effective_user: u32) -> c_int {
    credential_update(|process| {
        process.set_real_and_effective_user(
            (real_user != u32::MAX).then_some(real_user),
            (effective_user != u32::MAX).then_some(effective_user),
        )
    })
}

#[no_mangle]
pub extern "C" fn setsid() -> i32 {
    process_query(|process| process.create_session())
}

#[no_mangle]
pub extern "C" fn setuid(user: u32) -> c_int {
    credential_update(|process| process.set_user_id(user))
}

#[no_mangle]
pub extern "C" fn nice(_increment: c_int) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn getopt(
    argument_count: c_int,
    arguments: *const *mut c_char,
    options: *const c_char,
) -> c_int {
    unsafe { core::ptr::addr_of_mut!(optarg).write_volatile(core::ptr::null_mut()) };
    if argument_count <= 0 || arguments.is_null() || options.is_null() {
        return -1;
    }

    let mut argument_index = unsafe { core::ptr::addr_of!(optind).read_volatile() };
    if argument_index <= 0 {
        argument_index = 1;
        unsafe { core::ptr::addr_of_mut!(optind).write_volatile(argument_index) };
        unsafe { clear_next_option() };
    }
    if argument_index >= argument_count {
        unsafe { clear_next_option() };
        return -1;
    }

    let mut current_option = unsafe { core::ptr::addr_of!(NEXT_OPTION).read_volatile() };
    let saved_index = unsafe { core::ptr::addr_of!(NEXT_OPTION_INDEX).read_volatile() };
    if !current_option.is_null() && saved_index != argument_index {
        unsafe { clear_next_option() };
        current_option = core::ptr::null();
    }

    if current_option.is_null() {
        let current_argument = unsafe { read_volatile(arguments.add(argument_index as usize)) };
        if current_argument.is_null()
            || unsafe { option_byte(current_argument, 0) != b'-' }
            || unsafe { option_byte(current_argument, 1) == 0 }
        {
            unsafe { clear_next_option() };
            return -1;
        }
        if unsafe { option_byte(current_argument, 1) == b'-' && option_byte(current_argument, 2) == 0 }
        {
            unsafe {
                core::ptr::addr_of_mut!(optind).write_volatile(argument_index + 1);
                clear_next_option();
            }
            return -1;
        }
        current_option = unsafe { current_argument.add(1) };
    }

    let option = unsafe { option_byte(current_option, 0) };
    let remaining_options = unsafe { current_option.add(1) };
    let mut option_position = usize::from(unsafe { option_byte(options, 0) == b':' });
    while unsafe { option_byte(options, option_position) != 0 }
        && unsafe { option_byte(options, option_position) != option }
    {
        option_position += 1;
    }
    let found = unsafe { option_byte(options, option_position) == option && option != b':' };
    if !found {
        unsafe { core::ptr::addr_of_mut!(optopt).write_volatile(option as c_int) };
        if unsafe { option_byte(remaining_options, 0) == 0 } {
            unsafe {
                core::ptr::addr_of_mut!(optind).write_volatile(argument_index + 1);
                clear_next_option();
            }
        } else {
            unsafe { save_next_option(remaining_options, argument_index) };
        }
        return b'?' as c_int;
    }

    if unsafe { option_byte(options, option_position + 1) == b':' } {
        if unsafe { option_byte(remaining_options, 0) != 0 } {
            unsafe {
                core::ptr::addr_of_mut!(optarg).write_volatile(remaining_options.cast_mut());
                core::ptr::addr_of_mut!(optind).write_volatile(argument_index + 1);
                clear_next_option();
            }
        } else if argument_index + 1 < argument_count {
            let value = unsafe { read_volatile(arguments.add((argument_index + 1) as usize)) };
            unsafe {
                core::ptr::addr_of_mut!(optarg).write_volatile(value);
                core::ptr::addr_of_mut!(optind).write_volatile(argument_index + 2);
                clear_next_option();
            }
        } else {
            unsafe {
                core::ptr::addr_of_mut!(optopt).write_volatile(option as c_int);
                core::ptr::addr_of_mut!(optind).write_volatile(argument_index + 1);
                clear_next_option();
            }
            return if unsafe { option_byte(options, 0) == b':' } {
                b':' as c_int
            } else {
                b'?' as c_int
            };
        }
    } else if unsafe { option_byte(remaining_options, 0) == 0 } {
        unsafe {
            core::ptr::addr_of_mut!(optind).write_volatile(argument_index + 1);
            clear_next_option();
        }
    } else {
        unsafe { save_next_option(remaining_options, argument_index) };
    }
    option as c_int
}

#[no_mangle]
pub extern "C" fn isatty(file_descriptor: c_int) -> c_int {
    match terminal_descriptor_status(file_descriptor) {
        Ok(true) => 1,
        Ok(false) => {
            set_errno(ENOTTY);
            0
        }
        Err(error) => {
            set_errno(error);
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn tcgetpgrp(file_descriptor: c_int) -> i32 {
    match terminal_descriptor_status(file_descriptor) {
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
    process_query(Process::terminal_foreground_process_group)
}

#[no_mangle]
pub extern "C" fn tcsetpgrp(file_descriptor: c_int, process_group: i32) -> c_int {
    match terminal_descriptor_status(file_descriptor) {
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
    credential_update(|process| process.set_terminal_foreground_process_group(process_group))
}

#[no_mangle]
pub unsafe extern "C" fn ctermid(buffer: *mut c_char) -> *mut c_char {
    const TERMINAL_NAME: &[u8] = b"/dev/tty\0";
    if buffer.is_null() {
        return TERMINAL_NAME.as_ptr() as *mut c_char;
    }
    for (index, byte) in TERMINAL_NAME.iter().copied().enumerate() {
        unsafe { write_volatile(buffer.cast::<u8>().add(index), byte) };
    }
    buffer
}

#[no_mangle]
pub unsafe extern "C" fn ttyname(file_descriptor: c_int) -> *mut c_char {
    const TERMINAL_NAME: &[u8] = b"/dev/console\0";
    match terminal_descriptor_status(file_descriptor) {
        Ok(true) => return TERMINAL_NAME.as_ptr() as *mut c_char,
        Ok(false) => set_errno(ENOTTY),
        Err(error) => set_errno(error),
    }
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn ttyname_r(
    file_descriptor: c_int,
    buffer: *mut c_char,
    size: usize,
) -> c_int {
    const TERMINAL_NAME: &[u8] = b"/dev/console\0";
    match terminal_descriptor_status(file_descriptor) {
        Ok(true) => {}
        Ok(false) => return ENOTTY,
        Err(error) => return error,
    }
    if buffer.is_null() {
        return EFAULT;
    }
    if size < TERMINAL_NAME.len() {
        return ERANGE;
    }
    for (index, byte) in TERMINAL_NAME.iter().copied().enumerate() {
        unsafe { write_volatile(buffer.cast::<u8>().add(index), byte) };
    }
    0
}

#[no_mangle]
pub extern "C" fn alarm(seconds: u32) -> u32 {
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_kernel_errno(error);
            return 0;
        }
    };
    let now_ns = crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns);
    process.set_alarm(now_ns, seconds)
}

#[no_mangle]
pub extern "C" fn pause() -> c_int {
    let mask = match crate::kernel::runtime::runtime_posix::signal::current_signal_mask() {
        Ok(mask) => mask,
        Err(error) => {
            set_kernel_errno(error);
            return -1;
        }
    };
    let mut signal_set = [0u64; 16];
    signal_set[0] = mask.bits();
    unsafe {
        crate::kernel::runtime::runtime_posix::signal::sigsuspend(
            signal_set.as_ptr().cast::<c_void>(),
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn sleep(seconds: u32) -> u32 {
    let request = Timespec {
        seconds: seconds as c_long,
        nanoseconds: 0,
    };
    let mut remaining = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    if unsafe { nanosleep(&request, &mut remaining) } == 0 {
        0
    } else {
        (remaining.seconds.max(0) as u64 + u64::from(remaining.nanoseconds > 0))
            .min(u32::MAX as u64) as u32
    }
}

#[no_mangle]
pub unsafe extern "C" fn swab(source: *const c_void, destination: *mut c_void, size: isize) {
    if source.is_null() || destination.is_null() || size < 2 {
        return;
    }
    let source = source.cast::<u8>();
    let destination = destination.cast::<u8>();
    let mut index = 0;
    while index + 1 < size as usize {
        let first = unsafe { read_volatile(source.add(index)) };
        let second = unsafe { read_volatile(source.add(index + 1)) };
        unsafe {
            write_volatile(destination.add(index), second);
            write_volatile(destination.add(index + 1), first);
        }
        index += 2;
    }
}

#[no_mangle]
pub unsafe extern "C" fn link(existing_path: *const c_char, new_path: *const c_char) -> c_int {
    unsafe { kz_rust_link(existing_path, new_path) }
}

#[no_mangle]
pub unsafe extern "C" fn linkat(
    old_directory_descriptor: c_int,
    old_path: *const c_char,
    new_directory_descriptor: c_int,
    new_path: *const c_char,
    flags: c_int,
) -> c_int {
    unsafe {
        kz_rust_linkat(
            old_directory_descriptor,
            old_path,
            new_directory_descriptor,
            new_path,
            flags,
        )
    }
}

#[no_mangle]
pub unsafe extern "C" fn symlink(_target: *const c_char, _link_path: *const c_char) -> c_int {
    unsafe { kz_rust_symlink(_target, _link_path) }
}

#[no_mangle]
pub unsafe extern "C" fn symlinkat(
    target: *const c_char,
    directory_descriptor: c_int,
    link_path: *const c_char,
) -> c_int {
    unsafe { kz_rust_symlinkat(target, directory_descriptor, link_path) }
}

#[no_mangle]
pub unsafe extern "C" fn readlink(
    path: *const c_char,
    buffer: *mut c_char,
    size: usize,
) -> isize {
    unsafe { kz_rust_readlink(path, buffer.cast(), size) }
}

#[no_mangle]
pub unsafe extern "C" fn readlinkat(
    directory_descriptor: c_int,
    path: *const c_char,
    buffer: *mut c_char,
    size: usize,
) -> isize {
    unsafe { kz_rust_readlinkat(directory_descriptor, path, buffer.cast(), size) }
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat(
    directory_descriptor: c_int,
    path: *const c_char,
    flags: c_int,
) -> c_int {
    unsafe { kz_rust_unlinkat(directory_descriptor, path, flags) }
}

// FIXME: The runtime VFS has no per-file configuration query interface.
#[no_mangle]
pub unsafe extern "C" fn pathconf(_path: *const c_char, _name: c_int) -> c_long {
    fail_long()
}

#[no_mangle]
pub extern "C" fn fpathconf(_file_descriptor: c_int, _name: c_int) -> c_long {
    fail_long()
}

#[no_mangle]
pub extern "C" fn lockf(file_descriptor: c_int, command: c_int, length: i64) -> c_int {
    const F_ULOCK: c_int = 0;
    const F_LOCK: c_int = 1;
    const F_TLOCK: c_int = 2;
    const F_TEST: c_int = 3;
    let mut lock = crate::kernel::runtime::runtime_libc::filesystem::FileLockRecord {
        lock_type: 1,
        whence: 1,
        start: 0,
        length,
        process_id: 0,
    };
    let result = match command {
        F_ULOCK => {
            lock.lock_type = 2;
            crate::kernel::runtime::runtime_libc::filesystem::set_record_lock(
                file_descriptor,
                &lock,
                false,
            )
        }
        F_LOCK => crate::kernel::runtime::runtime_libc::filesystem::set_record_lock(
            file_descriptor,
            &lock,
            true,
        ),
        F_TLOCK => crate::kernel::runtime::runtime_libc::filesystem::set_record_lock(
            file_descriptor,
            &lock,
            false,
        ),
        F_TEST => crate::kernel::runtime::runtime_libc::filesystem::query_record_lock(
            file_descriptor,
            &mut lock,
        )
        .and_then(|()| {
            if lock.lock_type == 2 {
                Ok(())
            } else {
                Err(as_errno(KernelError::Acces))
            }
        }),
        _ => return fail_int(),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pread(
    file_descriptor: c_int,
    buffer: *mut c_void,
    size: usize,
    offset: i64,
) -> isize {
    if buffer.is_null() && size != 0 {
        set_errno(EFAULT);
        return -1;
    }
    if size > isize::MAX as usize {
        set_errno(EINVAL);
        return -1;
    }
    if crate::kernel::runtime::runtime_libc::network::is_socket_fd(file_descriptor)
        || crate::kernel::runtime::runtime_libc::network::is_packet_fd(file_descriptor)
    {
        set_kernel_errno(KernelError::Spipe);
        return -1;
    }
    match crate::kernel::runtime::runtime_libc::filesystem::positioned_read(
        file_descriptor,
        buffer.cast(),
        size,
        offset,
    ) {
        Ok(amount) => amount as isize,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn pwrite(
    file_descriptor: c_int,
    buffer: *const c_void,
    size: usize,
    offset: i64,
) -> isize {
    if buffer.is_null() && size != 0 {
        set_errno(EFAULT);
        return -1;
    }
    if size > isize::MAX as usize {
        set_errno(EINVAL);
        return -1;
    }
    if crate::kernel::runtime::runtime_libc::network::is_socket_fd(file_descriptor)
        || crate::kernel::runtime::runtime_libc::network::is_packet_fd(file_descriptor)
    {
        set_kernel_errno(KernelError::Spipe);
        return -1;
    }
    match crate::kernel::runtime::runtime_libc::filesystem::positioned_write(
        file_descriptor,
        buffer.cast(),
        size,
        offset,
    ) {
        Ok(amount) => amount as isize,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

// FIXME: The kernel has no process environment, account database, or password cipher.
#[no_mangle]
pub unsafe extern "C" fn crypt(_key: *const c_char, _salt: *const c_char) -> *mut c_char {
    fail_pointer()
}

#[no_mangle]
pub unsafe extern "C" fn encrypt(_block: *mut c_char, _flag: c_int) {
    set_errno(ENOSYS);
}

#[no_mangle]
pub extern "C" fn sync() {
}