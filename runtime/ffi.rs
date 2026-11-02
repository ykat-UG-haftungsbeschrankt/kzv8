/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::c_void;

use crate::event_completion::Completion;
use crate::event_loop::{EventFlags, EventLoop, EventSourceId, EventSourceToken, RawEvent};
use crate::event_timers::TimerId;

#[no_mangle]
pub unsafe extern "C" fn kz_host_console_write(data: *const u8, length: usize) -> i32 {
    if length > 512 || (data.is_null() && length != 0) {
        return -1;
    }
    let bytes = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(data, length)
    };
    if crate::console::write(bytes) {
        length as i32
    } else {
        -1
    }
}

#[no_mangle]
pub extern "C" fn kz_host_time_ns() -> u64 {
    match crate::arch::timer::now_ns() {
        Some(now_ns) if now_ns != 0 => now_ns,
        _ => crate::arch::fallback_time_ns(),
    }
}

#[no_mangle]
pub extern "C" fn kz_host_wall_time_ms() -> i64 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_poll(
    context: *mut c_void,
    output: *mut RawEvent,
) -> i32 {
    if context.is_null() || output.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let now_ns = crate::arch::timer::now_ns().unwrap_or_else(crate::arch::fallback_time_ns);
    let Some(event) = events.poll_raw(now_ns) else {
        return 0;
    };
    unsafe { output.write(event) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_schedule_timer(
    context: *mut c_void,
    delay_ns: u64,
    timer_id: *mut u64,
) -> i32 {
    if context.is_null() || timer_id.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let now_ns = crate::arch::timer::now_ns().unwrap_or_else(crate::arch::fallback_time_ns);
    let Some(deadline_ns) = now_ns.checked_add(delay_ns) else {
        return 0;
    };
    let Some(timer) = events.schedule_timer(deadline_ns) else {
        return 0;
    };
    unsafe { timer_id.write(timer.raw()) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_cancel_timer(
    context: *mut c_void,
    timer_id: u64,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.cancel_timer(TimerId::from_raw(timer_id)) as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_register(
    context: *mut c_void,
    source_id: u64,
    interest: u32,
    data: u64,
    token: *mut u64,
) -> i32 {
    if context.is_null() || token.is_null() {
        return 0;
    }
    let Some(interest) = EventFlags::from_bits(interest) else {
        return 0;
    };
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let Ok(source_token) = events.register(EventSourceId::new(source_id), interest, data) else {
        return 0;
    };
    unsafe { token.write(source_token.raw()) };
    1
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_modify(
    context: *mut c_void,
    token: u64,
    interest: u32,
    data: u64,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let Some(interest) = EventFlags::from_bits(interest) else {
        return 0;
    };
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.modify(EventSourceToken::from_raw(token), interest, data).is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_set_readiness(
    context: *mut c_void,
    token: u64,
    readiness: u32,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let Some(readiness) = EventFlags::from_bits(readiness) else {
        return 0;
    };
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events
        .set_readiness(EventSourceToken::from_raw(token), readiness)
        .is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_deregister(
    context: *mut c_void,
    token: u64,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.deregister(EventSourceToken::from_raw(token)).is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_complete(
    context: *mut c_void,
    id: u64,
    status: i32,
) -> i32 {
    if context.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    events.complete(Completion { id, status }).is_ok() as i32
}

#[no_mangle]
pub unsafe extern "C" fn kz_host_event_loop_next_deadline(
    context: *mut c_void,
    deadline_ns: *mut u64,
) -> i32 {
    if context.is_null() || deadline_ns.is_null() {
        return 0;
    }
    let events = unsafe { &mut *context.cast::<EventLoop<16>>() };
    let Some(deadline) = events.next_deadline() else {
        return 0;
    };
    unsafe { deadline_ns.write(deadline) };
    1
}