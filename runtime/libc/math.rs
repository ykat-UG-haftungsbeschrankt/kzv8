/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[no_mangle]
pub extern "C" fn nearbyint(value: f64) -> f64 {
    if value != value || value >= 4_503_599_627_370_496.0 || value <= -4_503_599_627_370_496.0 {
        return value;
    }
    let integral = value as i64;
    let fraction = value - integral as f64;
    if fraction > 0.5 || (fraction == 0.5 && integral & 1 != 0) {
        (integral + 1) as f64
    } else if fraction < -0.5 || (fraction == -0.5 && integral & 1 != 0) {
        (integral - 1) as f64
    } else {
        integral as f64
    }
}

#[no_mangle]
pub extern "C" fn nearbyintf(value: f32) -> f32 {
    if value != value || value >= 8_388_608.0 || value <= -8_388_608.0 {
        return value;
    }
    let integral = value as i32;
    let fraction = value - integral as f32;
    if fraction > 0.5 || (fraction == 0.5 && integral & 1 != 0) {
        (integral + 1) as f32
    } else if fraction < -0.5 || (fraction == -0.5 && integral & 1 != 0) {
        (integral - 1) as f32
    } else {
        integral as f32
    }
}

#[no_mangle]
pub unsafe extern "C" fn modf(value: f64, integral_part: *mut f64) -> f64 {
    if integral_part.is_null() {
        return value;
    }
    if value != value
        || value >= 9_223_372_036_854_775_808.0
        || value < -9_223_372_036_854_775_808.0
    {
        unsafe { integral_part.write(value) };
        return 0.0;
    }
    let integral = value as i64 as f64;
    unsafe { integral_part.write(integral) };
    value - integral
}

#[no_mangle]
pub unsafe extern "C" fn modff(value: f32, integral_part: *mut f32) -> f32 {
    if integral_part.is_null() {
        return value;
    }
    if value != value || value >= 2_147_483_648.0 || value < -2_147_483_648.0 {
        unsafe { integral_part.write(value) };
        return 0.0;
    }
    let integral = value as i32 as f32;
    unsafe { integral_part.write(integral) };
    value - integral
}

#[no_mangle]
pub extern "C" fn nextafter(value: f64, direction: f64) -> f64 {
    if value != value || direction != direction {
        return value + direction;
    }
    if value == direction {
        return direction;
    }
    if value == 0.0 {
        let sign = if direction < 0.0 { 1_u64 << 63 } else { 0 };
        return f64::from_bits(1 | sign);
    }
    let bits = value.to_bits();
    let next = if (value < direction) == (value > 0.0) {
        bits.wrapping_add(1)
    } else {
        bits.wrapping_sub(1)
    };
    f64::from_bits(next)
}

#[no_mangle]
pub extern "C" fn nextafterf(value: f32, direction: f32) -> f32 {
    if value != value || direction != direction {
        return value + direction;
    }
    if value == direction {
        return direction;
    }
    if value == 0.0 {
        let sign = if direction < 0.0 { 1_u32 << 31 } else { 0 };
        return f32::from_bits(1 | sign);
    }
    let bits = value.to_bits();
    let next = if (value < direction) == (value > 0.0) {
        bits.wrapping_add(1)
    } else {
        bits.wrapping_sub(1)
    };
    f32::from_bits(next)
}
