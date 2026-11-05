/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::read_volatile;
use super::errno::{EFAULT, EINVAL, ENOSYS};

const CLOCK_REALTIME: c_int = 0;
const CLOCK_MONOTONIC: c_int = 1;
const CLOCK_MONOTONIC_RAW: c_int = 4;
const TIMER_ABSTIME: c_int = 1;
const TIME_UTC: c_int = 1;

#[repr(C)]
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
    unavailable() as i64
}

#[no_mangle]
pub unsafe extern "C" fn clock_getcpuclockid(_process_id: i32, _clock_id: *mut c_int) -> c_int {
    ENOSYS
}

#[no_mangle]
pub unsafe extern "C" fn clock_getres(clock_id: c_int, resolution: *mut Timespec) -> c_int {
    if !matches!(clock_id, CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW) {
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
pub unsafe extern "C" fn clock_settime(_clock_id: c_int, _value: *const Timespec) -> c_int {
    unavailable()
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

#[no_mangle]
pub extern "C" fn timer_create(
    _clock_id: c_int,
    _event: *mut c_void,
    _timer: *mut usize,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn timer_delete(_timer: usize) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn timer_getoverrun(_timer: usize) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn timer_gettime(_timer: usize, _value: *mut c_void) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn timer_settime(
    _timer: usize,
    _flags: c_int,
    _value: *const c_void,
    _old_value: *mut c_void,
) -> c_int {
    unavailable()
}

#[no_mangle]
pub extern "C" fn tzset() {}