use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::{read_volatile, write_volatile};

use super::errno::{EBADF, EFAULT, EINVAL, ENAMETOOLONG, ENOSYS, ENOTTY};

const AT_FDCWD: c_int = -100;
const AT_REMOVEDIR: c_int = 0x200;
const CS_PATH: c_int = 0;
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
    fn nanosleep(request: *const Timespec, remaining: *mut Timespec) -> c_int;
    fn rmdir(path: *const c_char) -> c_int;
    fn uname(output: *mut c_void) -> c_int;
    fn unlink(path: *const c_char) -> c_int;
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
pub unsafe extern "C" fn chdir(_path: *const c_char) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn fchdir(_file_descriptor: c_int) -> c_int {
    fail_int()
}

// FIXME: The runtime VFS has no user/group ownership metadata.
#[no_mangle]
pub unsafe extern "C" fn chown(
    _path: *const c_char,
    _owner: u32,
    _group: u32,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn fchown(_file_descriptor: c_int, _owner: u32, _group: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn fchownat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _owner: u32,
    _group: u32,
    _flags: c_int,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn lchown(_path: *const c_char, _owner: u32, _group: u32) -> c_int {
    fail_int()
}

// FIXME: The runtime VFS has no descriptor duplication or pipe facility.
#[no_mangle]
pub extern "C" fn dup(_file_descriptor: c_int) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn dup2(_file_descriptor: c_int, _new_descriptor: c_int) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn dup3(
    _file_descriptor: c_int,
    _new_descriptor: c_int,
    _flags: c_int,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn pipe(_file_descriptors: *mut c_int) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn pipe2(_file_descriptors: *mut c_int, _flags: c_int) -> c_int {
    fail_int()
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

// FIXME: The kernel has no process table, process-creation API, or executable loader.
#[no_mangle]
pub extern "C" fn fork() -> i32 {
    fail_int()
}

#[no_mangle]
pub extern "C" fn _Fork() -> i32 {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn execv(_path: *const c_char, _arguments: *const *mut c_char) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn execve(
    _path: *const c_char,
    _arguments: *const *mut c_char,
    _environment: *const *mut c_char,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn execvp(_file: *const c_char, _arguments: *const *mut c_char) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn fexecve(
    _file_descriptor: c_int,
    _arguments: *const *mut c_char,
    _environment: *const *mut c_char,
) -> c_int {
    fail_int()
}

// FIXME: The runtime VFS has no synchronization with persistent storage.
#[no_mangle]
pub extern "C" fn fdatasync(_file_descriptor: c_int) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn fsync(_file_descriptor: c_int) -> c_int {
    fail_int()
}

// FIXME: The runtime VFS has no file-resize operation.
#[no_mangle]
pub extern "C" fn ftruncate(_file_descriptor: c_int, _length: i64) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn truncate(_path: *const c_char, _length: i64) -> c_int {
    fail_int()
}

// FIXME: The runtime has no kernel entropy source.
#[no_mangle]
pub unsafe extern "C" fn getentropy(buffer: *mut c_void, size: usize) -> c_int {
    if buffer.is_null() && size != 0 {
        set_errno(EFAULT);
        return -1;
    }
    fail_int()
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
    0
}

#[no_mangle]
pub extern "C" fn geteuid() -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn getgid() -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn getuid() -> u32 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn getgroups(group_count: c_int, _groups: *mut u32) -> c_int {
    if group_count < 0 {
        set_errno(EINVAL);
        -1
    } else {
        0
    }
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
    unsafe {
        real_group.write(0);
        effective_group.write(0);
        saved_group.write(0);
    }
    0
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
    unsafe {
        real_user.write(0);
        effective_user.write(0);
        saved_user.write(0);
    }
    0
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
pub unsafe extern "C" fn getpgid(_process: i32) -> i32 {
    fail_int()
}

#[no_mangle]
pub extern "C" fn getpgrp() -> i32 {
    fail_int()
}

#[no_mangle]
pub extern "C" fn getppid() -> i32 {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn getsid(_process: i32) -> i32 {
    fail_int()
}

// FIXME: The kernel has no process credentials or process-group facility.
#[no_mangle]
pub extern "C" fn setegid(_group: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn seteuid(_user: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setgid(_group: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setpgid(_process: i32, _group: i32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setpgrp() -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setregid(_real_group: u32, _effective_group: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setresgid(_real_group: u32, _effective_group: u32, _saved_group: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setresuid(_real_user: u32, _effective_user: u32, _saved_user: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setreuid(_real_user: u32, _effective_user: u32) -> c_int {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setsid() -> i32 {
    fail_int()
}

#[no_mangle]
pub extern "C" fn setuid(_user: u32) -> c_int {
    fail_int()
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

// FIXME: The kernel has no TTY line discipline or terminal process groups.
#[no_mangle]
pub extern "C" fn isatty(_file_descriptor: c_int) -> c_int {
    set_errno(if _file_descriptor < 0 { EBADF } else { ENOTTY });
    0
}

#[no_mangle]
pub extern "C" fn tcgetpgrp(_file_descriptor: c_int) -> i32 {
    set_errno(ENOTTY);
    -1
}

#[no_mangle]
pub extern "C" fn tcsetpgrp(_file_descriptor: c_int, _process_group: i32) -> c_int {
    set_errno(ENOTTY);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn ctermid(_buffer: *mut c_char) -> *mut c_char {
    fail_pointer()
}

#[no_mangle]
pub unsafe extern "C" fn ttyname(_file_descriptor: c_int) -> *mut c_char {
    set_errno(ENOTTY);
    core::ptr::null_mut()
}

#[no_mangle]
pub unsafe extern "C" fn ttyname_r(
    _file_descriptor: c_int,
    _buffer: *mut c_char,
    _size: usize,
) -> c_int {
    ENOTTY
}

// FIXME: The kernel has no signal timer or interruptible signal-wait facility.
#[no_mangle]
pub extern "C" fn alarm(_seconds: u32) -> u32 {
    set_errno(ENOSYS);
    0
}

#[no_mangle]
pub extern "C" fn pause() -> c_int {
    fail_int()
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
        remaining.seconds.max(0) as u32
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

// FIXME: The runtime VFS has no hard-link or symbolic-link operations.
#[no_mangle]
pub unsafe extern "C" fn link(_existing_path: *const c_char, _new_path: *const c_char) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn linkat(
    _old_directory_descriptor: c_int,
    _old_path: *const c_char,
    _new_directory_descriptor: c_int,
    _new_path: *const c_char,
    _flags: c_int,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn symlink(_target: *const c_char, _link_path: *const c_char) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn symlinkat(
    _target: *const c_char,
    _directory_descriptor: c_int,
    _link_path: *const c_char,
) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn readlink(
    _path: *const c_char,
    _buffer: *mut c_char,
    _size: usize,
) -> isize {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn readlinkat(
    _directory_descriptor: c_int,
    _path: *const c_char,
    _buffer: *mut c_char,
    _size: usize,
) -> isize {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn unlinkat(
    directory_descriptor: c_int,
    path: *const c_char,
    flags: c_int,
) -> c_int {
    if directory_descriptor != AT_FDCWD {
        return fail_int();
    }
    match flags {
        0 => unsafe { unlink(path) },
        AT_REMOVEDIR => unsafe { rmdir(path) },
        _ => {
            set_errno(EINVAL);
            -1
        }
    }
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
pub extern "C" fn lockf(_file_descriptor: c_int, _command: c_int, _length: i64) -> c_int {
    fail_int()
}

#[no_mangle]
pub unsafe extern "C" fn pread(
    _file_descriptor: c_int,
    _buffer: *mut c_void,
    _size: usize,
    _offset: i64,
) -> isize {
    set_errno(ENOSYS);
    -1
}

#[no_mangle]
pub unsafe extern "C" fn pwrite(
    _file_descriptor: c_int,
    _buffer: *const c_void,
    _size: usize,
    _offset: i64,
) -> isize {
    set_errno(ENOSYS);
    -1
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

// FIXME: The kernel has no persistent filesystem to synchronize.
#[no_mangle]
pub extern "C" fn sync() {
    set_errno(ENOSYS);
}