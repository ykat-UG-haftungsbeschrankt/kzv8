use core::ffi::{c_char, c_int, c_void};

static DAYS: [&[u8]; 7] = [
    b"Sunday\0",
    b"Monday\0",
    b"Tuesday\0",
    b"Wednesday\0",
    b"Thursday\0",
    b"Friday\0",
    b"Saturday\0",
];

static ABBREVIATED_DAYS: [&[u8]; 7] = [
    b"Sun\0",
    b"Mon\0",
    b"Tue\0",
    b"Wed\0",
    b"Thu\0",
    b"Fri\0",
    b"Sat\0",
];

static MONTHS: [&[u8]; 12] = [
    b"January\0",
    b"February\0",
    b"March\0",
    b"April\0",
    b"May\0",
    b"June\0",
    b"July\0",
    b"August\0",
    b"September\0",
    b"October\0",
    b"November\0",
    b"December\0",
];

static ABBREVIATED_MONTHS: [&[u8]; 12] = [
    b"Jan\0",
    b"Feb\0",
    b"Mar\0",
    b"Apr\0",
    b"May\0",
    b"Jun\0",
    b"Jul\0",
    b"Aug\0",
    b"Sep\0",
    b"Oct\0",
    b"Nov\0",
    b"Dec\0",
];

fn pointer(value: &'static [u8]) -> *mut c_char {
    value.as_ptr().cast_mut().cast()
}

fn langinfo(item: c_int) -> *mut c_char {
    match item {
        1 => pointer(b"UTF-8\0"),
        2 => pointer(b"%a %b %e %H:%M:%S %Y\0"),
        3 => pointer(b"%m/%d/%y\0"),
        4 => pointer(b"%H:%M:%S\0"),
        5 => pointer(b"%I:%M:%S %p\0"),
        6 => pointer(b"AM\0"),
        7 => pointer(b"PM\0"),
        8..=14 => pointer(DAYS[(item - 8) as usize]),
        15..=21 => pointer(ABBREVIATED_DAYS[(item - 15) as usize]),
        22..=33 => pointer(MONTHS[(item - 22) as usize]),
        34..=45 | 58..=69 | 70..=74 => pointer(b"\0"),
        46..=57 => pointer(ABBREVIATED_MONTHS[(item - 46) as usize]),
        75 => pointer(b".\0"),
        76 => pointer(b"\0"),
        77 => pointer(b"^[yY]\0"),
        78 => pointer(b"^[nN]\0"),
        79 => pointer(b"\0"),
        _ => pointer(b"\0"),
    }
}

#[no_mangle]
pub extern "C" fn nl_langinfo(item: c_int) -> *mut c_char {
    langinfo(item)
}

#[no_mangle]
pub unsafe extern "C" fn nl_langinfo_l(item: c_int, _locale: *mut c_void) -> *mut c_char {
    langinfo(item)
}