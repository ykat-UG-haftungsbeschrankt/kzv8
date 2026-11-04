use core::ffi::{c_char, c_int, c_void};
use super::errno::ENOSYS;

macro_rules! unsupported_spawn {
    ($name:ident($($argument:ident: $argument_type:ty),* $(,)?)) => {
        #[no_mangle]
        pub unsafe extern "C" fn $name($($argument: $argument_type),*) -> c_int {
            ENOSYS
        }
    };
}

// FIXME: The kernel has no process creation, executable loader, or child-process model.
unsupported_spawn!(posix_spawn(
    _process_id: *mut i32,
    _path: *const c_char,
    _file_actions: *const c_void,
    _attributes: *const c_void,
    _arguments: *const *mut c_char,
    _environment: *const *mut c_char,
));

unsupported_spawn!(posix_spawnp(
    _process_id: *mut i32,
    _file: *const c_char,
    _file_actions: *const c_void,
    _attributes: *const c_void,
    _arguments: *const *mut c_char,
    _environment: *const *mut c_char,
));

unsupported_spawn!(posix_spawn_file_actions_addchdir(_actions: *mut c_void, _path: *const c_char));
unsupported_spawn!(posix_spawn_file_actions_addclose(_actions: *mut c_void, _file_descriptor: c_int));
unsupported_spawn!(posix_spawn_file_actions_adddup2(_actions: *mut c_void, _source: c_int, _destination: c_int));
unsupported_spawn!(posix_spawn_file_actions_addfchdir(_actions: *mut c_void, _file_descriptor: c_int));
unsupported_spawn!(posix_spawn_file_actions_addopen(
    _actions: *mut c_void,
    _file_descriptor: c_int,
    _path: *const c_char,
    _flags: c_int,
    _mode: u32,
));
unsupported_spawn!(posix_spawn_file_actions_destroy(_actions: *mut c_void));
unsupported_spawn!(posix_spawn_file_actions_init(_actions: *mut c_void));

unsupported_spawn!(posix_spawnattr_destroy(_attributes: *mut c_void));
unsupported_spawn!(posix_spawnattr_getflags(_attributes: *const c_void, _flags: *mut i16));
unsupported_spawn!(posix_spawnattr_getpgroup(_attributes: *const c_void, _process_group: *mut i32));
unsupported_spawn!(posix_spawnattr_getschedparam(_attributes: *const c_void, _parameters: *mut c_void));
unsupported_spawn!(posix_spawnattr_getschedpolicy(_attributes: *const c_void, _policy: *mut c_int));
unsupported_spawn!(posix_spawnattr_getsigdefault(_attributes: *const c_void, _signal_set: *mut c_void));
unsupported_spawn!(posix_spawnattr_getsigmask(_attributes: *const c_void, _signal_set: *mut c_void));
unsupported_spawn!(posix_spawnattr_init(_attributes: *mut c_void));
unsupported_spawn!(posix_spawnattr_setflags(_attributes: *mut c_void, _flags: i16));
unsupported_spawn!(posix_spawnattr_setpgroup(_attributes: *mut c_void, _process_group: i32));
unsupported_spawn!(posix_spawnattr_setschedparam(_attributes: *mut c_void, _parameters: *const c_void));
unsupported_spawn!(posix_spawnattr_setschedpolicy(_attributes: *mut c_void, _policy: c_int));
unsupported_spawn!(posix_spawnattr_setsigdefault(_attributes: *mut c_void, _signal_set: *const c_void));
unsupported_spawn!(posix_spawnattr_setsigmask(_attributes: *mut c_void, _signal_set: *const c_void));