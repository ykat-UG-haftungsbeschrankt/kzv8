/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use core::ffi::{c_char, c_int, c_long, c_longlong};
use super::errno::ENOSYS;

fn unavailable_f64() -> f64 {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    f64::NAN
}

fn unavailable_f32() -> f32 {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    f32::NAN
}

fn unavailable_int() -> c_int {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(ENOSYS);
    }
    0
}

// FIXME: The kernel has no libm backend; Rust also has no C-compatible long double ABI.
macro_rules! unsupported_unary {
    ($name:ident, $namef:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(_value: f64) -> f64 {
            unavailable_f64()
        }

        #[no_mangle]
        pub extern "C" fn $namef(_value: f32) -> f32 {
            unavailable_f32()
        }
    };
}

macro_rules! unsupported_binary {
    ($name:ident, $namef:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(_left: f64, _right: f64) -> f64 {
            unavailable_f64()
        }

        #[no_mangle]
        pub extern "C" fn $namef(_left: f32, _right: f32) -> f32 {
            unavailable_f32()
        }
    };
}

macro_rules! unsupported_rounding {
    ($name:ident, $namef:ident, $name_long:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(_value: f64) -> c_long {
            unavailable_int() as c_long
        }

        #[no_mangle]
        pub extern "C" fn $namef(_value: f32) -> c_long {
            unavailable_int() as c_long
        }

        #[no_mangle]
        pub extern "C" fn $name_long(_value: f64) -> c_long {
            unavailable_int() as c_long
        }
    };
}

unsupported_unary!(acos, acosf);
unsupported_unary!(acosh, acoshf);
unsupported_unary!(asin, asinf);
unsupported_unary!(asinh, asinhf);
unsupported_unary!(atan, atanf);
unsupported_unary!(atanh, atanhf);
unsupported_unary!(cbrt, cbrtf);
unsupported_unary!(cos, cosf);
unsupported_unary!(cosh, coshf);
unsupported_unary!(erf, erff);
unsupported_unary!(erfc, erfcf);
unsupported_unary!(exp, expf);
unsupported_unary!(exp2, exp2f);
unsupported_unary!(expm1, expm1f);
unsupported_unary!(log, logf);
unsupported_unary!(log10, log10f);
unsupported_unary!(log1p, log1pf);
unsupported_unary!(log2, log2f);
unsupported_unary!(logb, logbf);
#[no_mangle]
pub extern "C" fn nearbyint(value: f64) -> f64 {
    crate::runtime::runtime_libc::math::nearbyint(value)
}

#[no_mangle]
pub extern "C" fn nearbyintf(value: f32) -> f32 {
    crate::runtime::runtime_libc::math::nearbyintf(value)
}

unsupported_unary!(rint, rintf);
unsupported_unary!(sin, sinf);
unsupported_unary!(sinh, sinhf);
unsupported_unary!(tan, tanf);
unsupported_unary!(tanh, tanhf);
unsupported_unary!(tgamma, tgammaf);
unsupported_unary!(lgamma, lgammaf);

unsupported_binary!(atan2, atan2f);
unsupported_binary!(fdim, fdimf);
unsupported_binary!(fmod, fmodf);
unsupported_binary!(hypot, hypotf);
#[no_mangle]
pub extern "C" fn nextafter(value: f64, direction: f64) -> f64 {
    crate::runtime::runtime_libc::math::nextafter(value, direction)
}

#[no_mangle]
pub extern "C" fn nextafterf(value: f32, direction: f32) -> f32 {
    crate::runtime::runtime_libc::math::nextafterf(value, direction)
}

unsupported_binary!(pow, powf);
unsupported_binary!(remainder, remainderf);

#[no_mangle]
pub extern "C" fn ceil(value: f64) -> f64 {
    libm::ceil(value)
}

#[no_mangle]
pub extern "C" fn ceilf(value: f32) -> f32 {
    libm::ceilf(value)
}

#[no_mangle]
pub extern "C" fn floor(value: f64) -> f64 {
    libm::floor(value)
}

#[no_mangle]
pub extern "C" fn floorf(value: f32) -> f32 {
    libm::floorf(value)
}

#[no_mangle]
pub extern "C" fn trunc(value: f64) -> f64 {
    libm::trunc(value)
}

#[no_mangle]
pub extern "C" fn truncf(value: f32) -> f32 {
    libm::truncf(value)
}

#[no_mangle]
pub extern "C" fn round(value: f64) -> f64 {
    libm::round(value)
}

#[no_mangle]
pub extern "C" fn roundf(value: f32) -> f32 {
    libm::roundf(value)
}

#[no_mangle]
pub extern "C" fn fabs(value: f64) -> f64 {
    value.abs()
}

#[no_mangle]
pub extern "C" fn fabsf(value: f32) -> f32 {
    value.abs()
}

#[no_mangle]
pub extern "C" fn copysign(value: f64, sign: f64) -> f64 {
    value.copysign(sign)
}

#[no_mangle]
pub extern "C" fn copysignf(value: f32, sign: f32) -> f32 {
    value.copysign(sign)
}

#[no_mangle]
pub extern "C" fn fmax(left: f64, right: f64) -> f64 {
    left.max(right)
}

#[no_mangle]
pub extern "C" fn fmaxf(left: f32, right: f32) -> f32 {
    left.max(right)
}

#[no_mangle]
pub extern "C" fn fmin(left: f64, right: f64) -> f64 {
    left.min(right)
}

#[no_mangle]
pub extern "C" fn fminf(left: f32, right: f32) -> f32 {
    left.min(right)
}

#[no_mangle]
pub extern "C" fn sqrt(value: f64) -> f64 {
    libm::sqrt(value)
}

#[no_mangle]
pub extern "C" fn sqrtf(value: f32) -> f32 {
    libm::sqrtf(value)
}

#[no_mangle]
pub extern "C" fn fma(_left: f64, _right: f64, _addend: f64) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn fmaf(_left: f32, _right: f32, _addend: f32) -> f32 {
    unavailable_f32()
}

#[no_mangle]
pub unsafe extern "C" fn frexp(value: f64, exponent: *mut c_int) -> f64 {
    if !exponent.is_null() {
        unsafe { exponent.write(0) };
    }
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    unavailable_f64()
}

#[no_mangle]
pub unsafe extern "C" fn frexpf(value: f32, exponent: *mut c_int) -> f32 {
    if !exponent.is_null() {
        unsafe { exponent.write(0) };
    }
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    unavailable_f32()
}

#[no_mangle]
pub extern "C" fn ldexp(_value: f64, _exponent: c_int) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn ldexpf(_value: f32, _exponent: c_int) -> f32 {
    unavailable_f32()
}

#[no_mangle]
pub extern "C" fn nan(_tag: *const c_char) -> f64 {
    f64::NAN
}

#[no_mangle]
pub extern "C" fn nanf(_tag: *const c_char) -> f32 {
    f32::NAN
}

#[no_mangle]
pub unsafe extern "C" fn remquo(
    _left: f64,
    _right: f64,
    quotient: *mut c_int,
) -> f64 {
    if !quotient.is_null() {
        unsafe { quotient.write(0) };
    }
    unavailable_f64()
}

#[no_mangle]
pub unsafe extern "C" fn remquof(
    _left: f32,
    _right: f32,
    quotient: *mut c_int,
) -> f32 {
    if !quotient.is_null() {
        unsafe { quotient.write(0) };
    }
    unavailable_f32()
}

#[no_mangle]
pub extern "C" fn scalbln(_value: f64, _exponent: c_long) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn scalblnf(_value: f32, _exponent: c_long) -> f32 {
    unavailable_f32()
}

#[no_mangle]
pub extern "C" fn scalbn(_value: f64, _exponent: c_int) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn scalbnf(_value: f32, _exponent: c_int) -> f32 {
    unavailable_f32()
}

#[no_mangle]
pub extern "C" fn ilogb(_value: f64) -> c_int {
    unavailable_int()
}

#[no_mangle]
pub extern "C" fn ilogbf(_value: f32) -> c_int {
    unavailable_int()
}

#[no_mangle]
pub extern "C" fn lrint(_value: f64) -> c_long {
    unavailable_int() as c_long
}

#[no_mangle]
pub extern "C" fn lrintf(_value: f32) -> c_long {
    unavailable_int() as c_long
}

#[no_mangle]
pub extern "C" fn llrint(_value: f64) -> c_longlong {
    unavailable_int() as c_longlong
}

#[no_mangle]
pub extern "C" fn llrintf(_value: f32) -> c_longlong {
    unavailable_int() as c_longlong
}

#[no_mangle]
pub extern "C" fn lround(_value: f64) -> c_long {
    unavailable_int() as c_long
}

#[no_mangle]
pub extern "C" fn lroundf(_value: f32) -> c_long {
    unavailable_int() as c_long
}

#[no_mangle]
pub extern "C" fn llround(_value: f64) -> c_longlong {
    unavailable_int() as c_longlong
}

#[no_mangle]
pub extern "C" fn llroundf(_value: f32) -> c_longlong {
    unavailable_int() as c_longlong
}

#[no_mangle]
pub extern "C" fn j0(_value: f64) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn j1(_value: f64) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn jn(_order: c_int, _value: f64) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn y0(_value: f64) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn y1(_value: f64) -> f64 {
    unavailable_f64()
}

#[no_mangle]
pub extern "C" fn yn(_order: c_int, _value: f64) -> f64 {
    unavailable_f64()
}

#[allow(non_upper_case_globals)]
#[no_mangle]
pub static mut signgam: c_int = 0;