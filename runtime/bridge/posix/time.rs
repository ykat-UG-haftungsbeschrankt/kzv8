/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::boxed::Box;
use core::ffi::{c_char, c_int, c_long, c_void};
use core::mem::{size_of, MaybeUninit};
use core::ptr::read_volatile;

use crate::kernel::arch::traits::TimerArchitecture as _;
use crate::kernel::dl::AddressSpace;
use crate::kernel::error::Error;
use crate::kernel::event::{EventLoop, EventLoopApi as _};
use crate::kernel::process::KernelObjectRef;
use crate::kernel::scheduler::sync::IrqSpinLock;
use crate::kernel::scheduler::thread::{vm::UserAddressSpace, ThreadAttributes, ThreadManager};
use crate::kernel::signal::{SignalInfo, MAX_SIGNAL_NUMBER, SI_TIMER};

use super::errno::{as_errno, EFAULT, EINVAL, ENOSYS, EPERM};

const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const CLOCK_PROCESS_CPUTIME_ID: c_int = 2;
const CLOCK_THREAD_CPUTIME_ID: c_int = 3;
const CLOCK_MONOTONIC_RAW: c_int = 4;
const CLOCKS_PER_SEC: u64 = 1_000_000;
const TIMER_ABSTIME: c_int = 1;
const TIME_UTC: c_int = 1;
const SIGEV_SIGNAL: c_int = 0;
const SIGEV_NONE: c_int = 1;
const SIGEV_THREAD: c_int = 2;
const SIGALRM: c_int = 14;
const TIMER_SLOT_COUNT: usize = 16;
const TIMER_SLOT_BITS: usize = 5;
const TIMER_SLOT_MASK: usize = (1 << TIMER_SLOT_BITS) - 1;
const TIMER_GENERATION_MASK: usize = usize::MAX >> TIMER_SLOT_BITS;
const PTHREAD_CREATE_DETACHED: c_int = 1;

#[repr(C)]
#[derive(Clone, Copy)]
union Sigval {
    integer: c_int,
    pointer: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Sigevent {
    notify: c_int,
    signal: c_int,
    value: Sigval,
    function: Option<unsafe extern "C" fn(Sigval)>,
    attributes: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Itimerspec {
    interval: Timespec,
    value: Timespec,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TimerThreadAttributes {
    stack_size: usize,
    detach_state: c_int,
    stack_address: usize,
}

#[derive(Clone, Copy)]
enum TimerNotification {
    None,
    Signal {
        signal: u8,
        value: usize,
    },
    Thread {
        function: unsafe extern "C" fn(Sigval),
        value: usize,
        attributes: TimerThreadAttributes,
    },
}

#[derive(Clone, Copy)]
struct PosixTimer {
    owner_pid: i32,
    generation: usize,
    active: bool,
    retired: bool,
    clock_id: c_int,
    event_id: u64,
    deadline_ns: u64,
    interval_ns: u64,
    overrun: c_int,
    notification: TimerNotification,
}

impl PosixTimer {
    const EMPTY: Self = Self {
        owner_pid: 0,
        generation: 0,
        active: false,
        retired: false,
        clock_id: 0,
        event_id: 0,
        deadline_ns: 0,
        interval_ns: 0,
        overrun: 0,
        notification: TimerNotification::None,
    };
}

struct TimerThreadContext {
    function: unsafe extern "C" fn(Sigval),
    value: usize,
}

static POSIX_TIMERS: IrqSpinLock<[PosixTimer; TIMER_SLOT_COUNT]> =
    IrqSpinLock::new([PosixTimer::EMPTY; TIMER_SLOT_COUNT]);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timespec {
    seconds: i64,
    nanoseconds: c_long,
}

#[repr(C)]
pub struct Tm {
    seconds: c_int,
    minutes: c_int,
    hours: c_int,
    month_day: c_int,
    month: c_int,
    year: c_int,
    week_day: c_int,
    year_day: c_int,
    daylight: c_int,
    offset: c_long,
    zone: *const c_char,
}

impl Tm {
    const EMPTY: Self = Self {
        seconds: 0,
        minutes: 0,
        hours: 0,
        month_day: 1,
        month: 0,
        year: 70,
        week_day: 4,
        year_day: 0,
        daylight: 0,
        offset: 0,
        zone: b"UTC\0".as_ptr().cast(),
    };
}

unsafe extern "C" {
    fn clock_gettime(clock_id: c_int, output: *mut Timespec) -> c_int;
    fn nanosleep(request: *const Timespec, remaining: *mut Timespec) -> c_int;
    fn localtime_r(time_value: *const i64, result: *mut Tm) -> *mut Tm;
    fn pthread_create(
        thread: *mut usize,
        attributes: *const c_void,
        routine: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_void>,
        argument: *mut c_void,
    ) -> c_int;
    fn pthread_detach(thread: usize) -> c_int;
}

static mut TM_BUFFER: Tm = Tm::EMPTY;
static mut ASCTIME_BUFFER: [u8; 26] = [0; 26];

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

fn floor_div(value: i64, divisor: i64) -> i64 {
    let quotient = value / divisor;
    let remainder = value % divisor;
    if remainder < 0 { quotient - 1 } else { quotient }
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted_days = days_since_epoch + 719_468;
    let era = floor_div(shifted_days, 146_097);
    let day_of_era = shifted_days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096)
        / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era
        - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 { month_prime + 3 } else { month_prime - 9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let adjusted_year = year - i64::from(month <= 2);
    let era = floor_div(adjusted_year, 400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[no_mangle]
pub unsafe extern "C" fn gmtime_r(time_value: *const i64, result: *mut Tm) -> *mut Tm {
    if time_value.is_null() || result.is_null() {
        set_errno(EFAULT);
        return core::ptr::null_mut();
    }
    let timestamp = unsafe { read_volatile(time_value) };
    let days = floor_div(timestamp, 86_400);
    let seconds_of_day = timestamp - days * 86_400;
    let (year, month, day) = civil_from_days(days);
    let weekday = (days + 4).rem_euclid(7);
    let year_start = days_from_civil(year, 1, 1);
    unsafe {
        result.write(Tm {
            seconds: (seconds_of_day % 60) as c_int,
            minutes: ((seconds_of_day / 60) % 60) as c_int,
            hours: (seconds_of_day / 3_600) as c_int,
            month_day: day as c_int,
            month: month as c_int - 1,
            year: (year - 1900).clamp(c_int::MIN as i64, c_int::MAX as i64) as c_int,
            week_day: weekday as c_int,
            year_day: (days - year_start) as c_int,
            daylight: 0,
            offset: 0,
            zone: b"UTC\0".as_ptr().cast(),
        });
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn gmtime(time_value: *const i64) -> *mut Tm {
    unsafe { gmtime_r(time_value, core::ptr::addr_of_mut!(TM_BUFFER)) }
}

#[no_mangle]
pub unsafe extern "C" fn localtime(time_value: *const i64) -> *mut Tm {
    unsafe { localtime_r(time_value, core::ptr::addr_of_mut!(TM_BUFFER)) }
}

#[no_mangle]
pub unsafe extern "C" fn asctime(time_value: *const Tm) -> *mut c_char {
    if time_value.is_null() {
        set_errno(EFAULT);
        return core::ptr::null_mut();
    }
    let weekdays: [&[u8; 4]; 7] = [b"Sun\0", b"Mon\0", b"Tue\0", b"Wed\0", b"Thu\0", b"Fri\0", b"Sat\0"];
    let months: [&[u8; 4]; 12] = [b"Jan\0", b"Feb\0", b"Mar\0", b"Apr\0", b"May\0", b"Jun\0", b"Jul\0", b"Aug\0", b"Sep\0", b"Oct\0", b"Nov\0", b"Dec\0"];
    let time_value = unsafe { &*time_value };
    if !(0..7).contains(&time_value.week_day) || !(0..12).contains(&time_value.month) {
        set_errno(EINVAL);
        return core::ptr::null_mut();
    }
    let output = core::ptr::addr_of_mut!(ASCTIME_BUFFER).cast::<u8>();
    unsafe {
        for index in 0..3 {
            output.add(index).write(weekdays[time_value.week_day as usize][index]);
            output.add(index + 4).write(months[time_value.month as usize][index]);
        }
        output.add(3).write(b' ');
        output.add(7).write(b' ');
        output.add(8).write(if time_value.month_day < 10 { b' ' } else { b'0' + (time_value.month_day / 10) as u8 });
        output.add(9).write(b'0' + (time_value.month_day % 10) as u8);
        output.add(10).write(b' ');
        write_two_digits(output.add(11), time_value.hours);
        output.add(13).write(b':');
        write_two_digits(output.add(14), time_value.minutes);
        output.add(16).write(b':');
        write_two_digits(output.add(17), time_value.seconds);
        output.add(19).write(b' ');
        let year = time_value.year.saturating_add(1900).clamp(0, 9999);
        output.add(20).write(b'0' + (year / 1000) as u8);
        output.add(21).write(b'0' + (year / 100 % 10) as u8);
        output.add(22).write(b'0' + (year / 10 % 10) as u8);
        output.add(23).write(b'0' + (year % 10) as u8);
        output.add(24).write(b'\n');
        output.add(25).write(0);
    }
    output.cast()
}

unsafe fn write_two_digits(output: *mut u8, value: c_int) {
    unsafe {
        output.write(b'0' + (value / 10) as u8);
        output.add(1).write(b'0' + (value % 10) as u8);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ctime(time_value: *const i64) -> *mut c_char {
    let broken_down = unsafe { localtime(time_value) };
    if broken_down.is_null() {
        return core::ptr::null_mut();
    }
    unsafe { asctime(broken_down) }
}

#[no_mangle]
pub extern "C" fn difftime(end_time: i64, start_time: i64) -> f64 {
    (end_time as f64) - (start_time as f64)
}

#[no_mangle]
pub unsafe extern "C" fn mktime(time_value: *mut Tm) -> i64 {
    if time_value.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let time_value = unsafe { &mut *time_value };
    let mut year = time_value.year as i64 + 1900;
    let mut month = time_value.month as i64 + 1;
    year += floor_div(month - 1, 12);
    month = (month - 1).rem_euclid(12) + 1;
    let days = days_from_civil(year, month, 1) + time_value.month_day as i64 - 1;
    let Some(seconds) = days
        .checked_mul(86_400)
        .and_then(|value| value.checked_add(time_value.hours as i64 * 3_600))
        .and_then(|value| value.checked_add(time_value.minutes as i64 * 60))
        .and_then(|value| value.checked_add(time_value.seconds as i64))
    else {
        set_errno(EINVAL);
        return -1;
    };
    let mut normalized = Tm::EMPTY;
    unsafe {
        gmtime_r(&seconds, &mut normalized);
        time_value.seconds = normalized.seconds;
        time_value.minutes = normalized.minutes;
        time_value.hours = normalized.hours;
        time_value.month_day = normalized.month_day;
        time_value.month = normalized.month;
        time_value.year = normalized.year;
        time_value.week_day = normalized.week_day;
        time_value.year_day = normalized.year_day;
        time_value.daylight = 0;
        time_value.offset = 0;
        time_value.zone = normalized.zone;
    }
    seconds
}

fn unavailable() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn clock() -> i64 {
    let cpu_time_ns = ThreadManager::current_process_cpu_time_ns();
    (cpu_time_ns / (1_000_000_000 / CLOCKS_PER_SEC)) as i64
}

#[no_mangle]
pub unsafe extern "C" fn clock_getcpuclockid(process_id: i32, clock_id: *mut c_int) -> c_int {
    if clock_id.is_null() {
        return as_errno(Error::Fault);
    }
    let clock_id_value = match crate::kernel::runtime::runtime_libc::posix::process_cpu_clock_id(
        process_id,
    ) {
        Ok(clock_id) => clock_id,
        Err(error) => return as_errno(error),
    };
    let caller = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => return as_errno(error),
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&clock_id_value as *const c_int).cast::<u8>(),
            size_of::<c_int>(),
        )
    };
    caller
        .address_space()
        .write(clock_id as usize, bytes)
        .map_or_else(as_errno, |_| 0)
}

#[no_mangle]
pub unsafe extern "C" fn clock_getres(clock_id: c_int, resolution: *mut Timespec) -> c_int {
    if clock_id < 0 {
        if let Err(error) = crate::kernel::runtime::runtime_libc::posix::process_cpu_time_for_clock_id(
            clock_id,
        ) {
            set_errno(as_errno(error));
            return -1;
        }
    } else if !matches!(
        clock_id,
        CLOCK_REALTIME
            | CLOCK_MONOTONIC
            | CLOCK_PROCESS_CPUTIME_ID
            | CLOCK_THREAD_CPUTIME_ID
            | CLOCK_MONOTONIC_RAW
    ) {
        set_errno(EINVAL);
        return -1;
    }
    if !resolution.is_null() {
        unsafe { resolution.write(Timespec { seconds: 0, nanoseconds: 1 }) };
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn clock_nanosleep(
    clock_id: c_int,
    flags: c_int,
    request: *const Timespec,
    remaining: *mut Timespec,
) -> c_int {
    if !matches!(clock_id, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW)
        || flags & !TIMER_ABSTIME != 0
        || request.is_null()
    {
        return EINVAL;
    }
    let mut relative = unsafe { request.read() };
    if flags & TIMER_ABSTIME != 0 {
        let mut now = Timespec { seconds: 0, nanoseconds: 0 };
        if unsafe { clock_gettime(clock_id, &mut now) } != 0 {
            return unsafe { crate::kernel::scheduler::thread::__errno_location().read() };
        }
        let requested_ns = relative.seconds.saturating_mul(1_000_000_000)
            .saturating_add(relative.nanoseconds as i64);
        let now_ns = now.seconds.saturating_mul(1_000_000_000)
            .saturating_add(now.nanoseconds as i64);
        let remaining_ns = requested_ns.saturating_sub(now_ns).max(0);
        relative.seconds = remaining_ns / 1_000_000_000;
        relative.nanoseconds = (remaining_ns % 1_000_000_000) as c_long;
    }
    if unsafe { nanosleep(&relative, remaining) } == 0 {
        0
    } else {
        unsafe { crate::kernel::scheduler::thread::__errno_location().read() }
    }
}

#[no_mangle]
pub unsafe extern "C" fn clock_settime(clock_id: c_int, value: *const Timespec) -> c_int {
    if clock_id != CLOCK_REALTIME {
        set_errno(EINVAL);
        return -1;
    }
    if value.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    let process = match ThreadManager::current_process() {
        Ok(process) => process,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    if !process.effective_user_is_root() {
        set_errno(EPERM);
        return -1;
    }
    let address_space = process.address_space();
    let value = match read_user_value::<Timespec>(&address_space, value) {
        Ok(value) => value,
        Err(error) => {
            set_errno(as_errno(error));
            return -1;
        }
    };
    if value.nanoseconds < 0 || value.nanoseconds >= 1_000_000_000 {
        set_errno(EINVAL);
        return -1;
    }
    let realtime_ns = i128::from(value.seconds)
        .saturating_mul(1_000_000_000)
        .saturating_add(i128::from(value.nanoseconds));
    match crate::kernel::runtime::runtime_libc::posix::set_realtime_clock(realtime_ns) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(as_errno(error));
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn timespec_get(value: *mut Timespec, base: c_int) -> c_int {
    if base != TIME_UTC || value.is_null() {
        return 0;
    }
    if unsafe { clock_gettime(CLOCK_REALTIME, value) } == 0 {
        base
    } else {
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn strftime(
    _output: *mut c_char,
    _output_size: usize,
    _format: *const c_char,
    _time_value: *const Tm,
) -> usize {
    unavailable();
    0
}

#[no_mangle]
pub unsafe extern "C" fn strptime(
    _input: *const c_char,
    _format: *const c_char,
    _result: *mut Tm,
) -> *mut c_char {
    unavailable();
    core::ptr::null_mut()
}

fn read_user_value<T: Copy>(address_space: &UserAddressSpace, pointer: *const T) -> Result<T, Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let mut value = MaybeUninit::<T>::uninit();
    let bytes = unsafe {
        core::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>())
    };
    address_space.read(pointer as usize, bytes)?;
    Ok(unsafe { value.assume_init() })
}

fn write_user_value<T: Copy>(
    address_space: &mut UserAddressSpace,
    pointer: *mut T,
    value: &T,
) -> Result<(), Error> {
    if pointer.is_null() {
        return Err(Error::Fault);
    }
    let bytes = unsafe {
        core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>())
    };
    address_space.write(pointer as usize, bytes)
}

fn monotonic_now_ns() -> u64 {
    crate::kernel::arch::TimerArchitecture::now_ns()
        .unwrap_or_else(crate::kernel::arch::TimerArchitecture::fallback_time_ns)
}

fn current_errno() -> c_int {
    unsafe { crate::kernel::scheduler::thread::__errno_location().read() }
}

fn supported_timer_clock(clock_id: c_int) -> bool {
    matches!(clock_id, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW)
}

fn timespec_duration_ns(value: Timespec) -> Result<u64, c_int> {
    if value.seconds < 0 || value.nanoseconds < 0 || value.nanoseconds >= 1_000_000_000 {
        return Err(EINVAL);
    }
    (value.seconds as u64)
        .checked_mul(1_000_000_000)
        .and_then(|seconds| seconds.checked_add(value.nanoseconds as u64))
        .ok_or(EINVAL)
}

fn clock_value_ns(clock_id: c_int) -> Result<i128, c_int> {
    let mut value = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    if unsafe { clock_gettime(clock_id, &mut value) } != 0 {
        return Err(current_errno());
    }
    if value.nanoseconds < 0 || value.nanoseconds >= 1_000_000_000 {
        return Err(EINVAL);
    }
    Ok(i128::from(value.seconds) * 1_000_000_000 + i128::from(value.nanoseconds))
}

fn timespec_from_ns(value: u64) -> Timespec {
    Timespec {
        seconds: (value / 1_000_000_000) as i64,
        nanoseconds: (value % 1_000_000_000) as c_long,
    }
}

fn timer_handle(index: usize, generation: usize) -> usize {
    (generation << TIMER_SLOT_BITS) | (index + 1)
}

fn timer_slot(handle: usize) -> Option<(usize, usize)> {
    let slot = handle & TIMER_SLOT_MASK;
    let generation = handle >> TIMER_SLOT_BITS;
    if slot == 0 || slot > TIMER_SLOT_COUNT || generation == 0 {
        return None;
    }
    Some((slot - 1, generation))
}

fn find_timer_index(timers: &[PosixTimer], pid: i32, handle: usize) -> Option<usize> {
    let (index, generation) = timer_slot(handle)?;
    let timer = timers.get(index)?;
    (timer.active && timer.owner_pid == pid && timer.generation == generation).then_some(index)
}

fn timer_value(timer: PosixTimer, now_ns: u64) -> Itimerspec {
    Itimerspec {
        interval: timespec_from_ns(timer.interval_ns),
        value: timespec_from_ns(if timer.event_id == 0 {
            0
        } else {
            timer.deadline_ns.saturating_sub(now_ns)
        }),
    }
}

fn current_process_address_space() -> Result<(i32, UserAddressSpace), c_int> {
    let process = ThreadManager::current_process().map_err(as_errno)?;
    Ok((process.pid(), process.address_space()))
}

unsafe extern "C" fn timer_thread_entry(argument: *mut c_void) -> *mut c_void {
    let context = unsafe { Box::from_raw(argument.cast::<TimerThreadContext>()) };
    unsafe {
        (context.function)(Sigval {
            pointer: context.value as *mut c_void,
        })
    };
    core::ptr::null_mut()
}

fn dispatch_timer_notification(pid: i32, notification: TimerNotification) {
    match notification {
        TimerNotification::None => {}
        TimerNotification::Signal { signal, value } => {
            if let Ok(process) = ThreadManager::current_process() {
                if process.pid() == pid {
                    let _ = process.queue_signal(SignalInfo::new(
                        signal,
                        SI_TIMER,
                        0,
                        0,
                        value,
                        0,
                    ));
                }
            }
        }
        TimerNotification::Thread {
            function,
            value,
            attributes,
        } => {
            let context = Box::new(TimerThreadContext { function, value });
            let argument = Box::into_raw(context).cast::<c_void>();
            let attributes = ThreadAttributes {
                stack_size: attributes.stack_size,
                detach_state: attributes.detach_state,
                stack_address: attributes.stack_address as *mut c_void,
                inherit_schedule: 0,
                scheduler_policy: 0,
                scheduler_priority: 0,
            };
            let mut thread = 0;
            let result = unsafe {
                pthread_create(
                    &mut thread,
                    (&attributes as *const ThreadAttributes).cast(),
                    Some(timer_thread_entry),
                    argument,
                )
            };
            if result != 0 {
                unsafe { drop(Box::from_raw(argument.cast::<TimerThreadContext>())) };
            } else if attributes.detach_state != PTHREAD_CREATE_DETACHED {
                let _ = unsafe { pthread_detach(thread) };
            }
        }
    }
}

pub(crate) fn dispatch_posix_timer_event(
    events: &mut EventLoop<16>,
    timer_id: u64,
    now_ns: u64,
) {
    let notification = {
        let mut timers = POSIX_TIMERS.lock();
        let Some(timer) = timers
            .iter_mut()
            .find(|timer| timer.active && timer.event_id == timer_id)
        else {
            return;
        };
        let notification = (timer.owner_pid, timer.notification);
        if timer.interval_ns == 0 {
            timer.event_id = 0;
            timer.deadline_ns = 0;
            timer.overrun = 0;
        } else {
            let missed = now_ns.saturating_sub(timer.deadline_ns) / timer.interval_ns;
            timer.overrun = missed.min(c_int::MAX as u64) as c_int;
            let periods = missed.saturating_add(1);
            let next_deadline = timer
                .interval_ns
                .checked_mul(periods)
                .and_then(|advance| timer.deadline_ns.checked_add(advance));
            if let Some(next_deadline) = next_deadline {
                if let Some(next_timer) = events.schedule_timer(next_deadline) {
                    timer.event_id = KernelObjectRef::from_arc(&next_timer).raw();
                    timer.deadline_ns = next_deadline;
                } else {
                    timer.event_id = 0;
                    timer.deadline_ns = 0;
                }
            } else {
                timer.event_id = 0;
                timer.deadline_ns = 0;
            }
        }
        Some(notification)
    };
    if let Some((pid, notification)) = notification {
        dispatch_timer_notification(pid, notification);
    }
}

fn timer_create_inner(
    clock_id: c_int,
    event: *mut c_void,
    timer: *mut usize,
) -> Result<(), c_int> {
    if !supported_timer_clock(clock_id) {
        return Err(EINVAL);
    }
    if timer.is_null() {
        return Err(EFAULT);
    }
    let (pid, mut address_space) = current_process_address_space()?;
    let notification = if event.is_null() {
        TimerNotification::Signal {
            signal: SIGALRM as u8,
            value: 0,
        }
    } else {
        let event = read_user_value::<Sigevent>(&address_space, event.cast()).map_err(as_errno)?;
        match event.notify {
            SIGEV_NONE => TimerNotification::None,
            SIGEV_SIGNAL => {
                if event.signal <= 0 || event.signal as u32 > u32::from(MAX_SIGNAL_NUMBER) {
                    return Err(EINVAL);
                }
                TimerNotification::Signal {
                    signal: event.signal as u8,
                    value: unsafe { event.value.pointer as usize },
                }
            }
            SIGEV_THREAD => {
                let Some(function) = event.function else {
                    return Err(EINVAL);
                };
                let attributes = if event.attributes.is_null() {
                    TimerThreadAttributes {
                        stack_size: 0,
                        detach_state: PTHREAD_CREATE_DETACHED,
                        stack_address: 0,
                    }
                } else {
                    read_user_value::<TimerThreadAttributes>(
                        &address_space,
                        event.attributes.cast(),
                    )
                    .map_err(as_errno)?
                };
                if !matches!(attributes.detach_state, 0 | PTHREAD_CREATE_DETACHED) {
                    return Err(EINVAL);
                }
                TimerNotification::Thread {
                    function,
                    value: unsafe { event.value.pointer as usize },
                    attributes,
                }
            }
            _ => return Err(EINVAL),
        }
    };

    let (index, generation, handle) = {
        let mut timers = POSIX_TIMERS.lock();
        let Some(index) = timers
            .iter()
            .position(|timer| !timer.active && !timer.retired)
        else {
            return Err(as_errno(Error::NoMem));
        };
        let generation = timers[index].generation + 1;
        let handle = timer_handle(index, generation);
        timers[index] = PosixTimer {
            owner_pid: pid,
            generation,
            active: true,
            retired: false,
            clock_id,
            event_id: 0,
            deadline_ns: 0,
            interval_ns: 0,
            overrun: 0,
            notification,
        };
        (index, generation, handle)
    };
    if let Err(error) = write_user_value(&mut address_space, timer, &handle) {
        let mut timers = POSIX_TIMERS.lock();
        if timers[index].active && timers[index].generation == generation {
            timers[index].active = false;
            timers[index].retired = generation == TIMER_GENERATION_MASK;
        }
        return Err(as_errno(error));
    }
    Ok(())
}

fn timer_delete_inner(handle: usize) -> Result<(), c_int> {
    let (pid, _) = current_process_address_space()?;
    let mut timers = POSIX_TIMERS.lock();
    let Some(index) = find_timer_index(&timers[..], pid, handle) else {
        return Err(EINVAL);
    };
    let timer_id = timers[index].event_id;
    if timer_id != 0 {
        let _ = crate::kernel::runtime::runtime_ffi::cancel_active_timer(timer_id);
    }
    timers[index].active = false;
    timers[index].retired = timers[index].generation == TIMER_GENERATION_MASK;
    timers[index].event_id = 0;
    timers[index].deadline_ns = 0;
    timers[index].interval_ns = 0;
    timers[index].overrun = 0;
    Ok(())
}

fn timer_gettime_inner(handle: usize, value: *mut c_void) -> Result<(), c_int> {
    if value.is_null() {
        return Err(EFAULT);
    }
    let (pid, mut address_space) = current_process_address_space()?;
    let timer = {
        let timers = POSIX_TIMERS.lock();
        let Some(index) = find_timer_index(&timers[..], pid, handle) else {
            return Err(EINVAL);
        };
        timers[index]
    };
    let result = timer_value(timer, monotonic_now_ns());
    write_user_value(&mut address_space, value.cast(), &result).map_err(as_errno)
}

fn timer_getoverrun_inner(handle: usize) -> Result<c_int, c_int> {
    let (pid, _) = current_process_address_space()?;
    let timers = POSIX_TIMERS.lock();
    let Some(index) = find_timer_index(&timers[..], pid, handle) else {
        return Err(EINVAL);
    };
    Ok(timers[index].overrun)
}

fn timer_settime_inner(
    handle: usize,
    flags: c_int,
    value: *const c_void,
    old_value: *mut c_void,
) -> Result<(), c_int> {
    if flags & !TIMER_ABSTIME != 0 {
        return Err(EINVAL);
    }
    if value.is_null() {
        return Err(EFAULT);
    }
    let (pid, mut address_space) = current_process_address_space()?;
    let requested = read_user_value::<Itimerspec>(&address_space, value.cast()).map_err(as_errno)?;
    let interval_ns = timespec_duration_ns(requested.interval)?;
    let value_ns = timespec_duration_ns(requested.value)?;
    let mut timers = POSIX_TIMERS.lock();
    let Some(index) = find_timer_index(&timers[..], pid, handle) else {
        return Err(EINVAL);
    };
    let previous = timers[index];
    let now_ns = monotonic_now_ns();
    if !old_value.is_null() {
        let previous_value = timer_value(previous, now_ns);
        write_user_value(&mut address_space, old_value.cast(), &previous_value)
            .map_err(as_errno)?;
    }

    let deadline_ns = if value_ns == 0 {
        None
    } else {
        let delay_ns = if flags & TIMER_ABSTIME != 0 {
            let clock_now = clock_value_ns(previous.clock_id)?;
            let delay = (i128::from(value_ns) - clock_now).max(0);
            u64::try_from(delay).map_err(|_| EINVAL)?
        } else {
            value_ns
        };
        Some(now_ns.checked_add(delay_ns).ok_or(EINVAL)?)
    };

    let cancelled = previous.event_id != 0
        && crate::kernel::runtime::runtime_ffi::cancel_active_timer(previous.event_id);
    let Some(deadline_ns) = deadline_ns else {
        timers[index].event_id = 0;
        timers[index].deadline_ns = 0;
        timers[index].interval_ns = interval_ns;
        timers[index].overrun = 0;
        return Ok(());
    };

    let Some(event_id) = crate::kernel::runtime::runtime_ffi::schedule_active_timer_at(deadline_ns)
    else {
        if previous.event_id != 0 && cancelled {
            if let Some(restored_id) =
                crate::kernel::runtime::runtime_ffi::schedule_active_timer_at(previous.deadline_ns)
            {
                timers[index].event_id = restored_id;
            } else {
                timers[index].event_id = 0;
                timers[index].deadline_ns = 0;
            }
        }
        return Err(as_errno(Error::NoMem));
    };
    timers[index].event_id = event_id;
    timers[index].deadline_ns = deadline_ns;
    timers[index].interval_ns = interval_ns;
    timers[index].overrun = 0;
    Ok(())
}

#[no_mangle]
pub extern "C" fn timer_create(
    clock_id: c_int,
    event: *mut c_void,
    timer: *mut usize,
) -> c_int {
    match timer_create_inner(clock_id, event, timer) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn timer_delete(timer: usize) -> c_int {
    match timer_delete_inner(timer) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn timer_getoverrun(timer: usize) -> c_int {
    match timer_getoverrun_inner(timer) {
        Ok(overrun) => overrun,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn timer_gettime(timer: usize, value: *mut c_void) -> c_int {
    match timer_gettime_inner(timer, value) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn timer_settime(
    timer: usize,
    flags: c_int,
    value: *const c_void,
    old_value: *mut c_void,
) -> c_int {
    match timer_settime_inner(timer, flags, value, old_value) {
        Ok(()) => 0,
        Err(error) => {
            set_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn tzset() {}