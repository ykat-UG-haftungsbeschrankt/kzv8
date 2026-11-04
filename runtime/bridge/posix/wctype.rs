use core::ffi::{c_char, c_int, c_void};
use core::ptr::read_volatile;

const ALNUM: u32 = 1;
const ALPHA: u32 = 2;
const BLANK: u32 = 3;
const CNTRL: u32 = 4;
const DIGIT: u32 = 5;
const GRAPH: u32 = 6;
const LOWER: u32 = 7;
const PRINT: u32 = 8;
const PUNCT: u32 = 9;
const SPACE: u32 = 10;
const UPPER: u32 = 11;
const XDIGIT: u32 = 12;
const TOLOWER: u32 = 1;
const TOUPPER: u32 = 2;

fn is_alpha(character: u32) -> bool {
    character <= 0x7f && (character as u8).is_ascii_alphabetic()
}

fn is_digit(character: u32) -> bool {
    character <= 0x7f && (character as u8).is_ascii_digit()
}

fn is_graph(character: u32) -> bool {
    (0x21..=0x7e).contains(&character)
}

fn classify(character: u32, kind: u32) -> bool {
    match kind {
        ALNUM => is_alpha(character) || is_digit(character),
        ALPHA => is_alpha(character),
        BLANK => matches!(character, 0x20 | 0x09),
        CNTRL => character < 0x20 || character == 0x7f,
        DIGIT => is_digit(character),
        GRAPH => is_graph(character),
        LOWER => character <= 0x7f && (character as u8).is_ascii_lowercase(),
        PRINT => (0x20..=0x7e).contains(&character),
        PUNCT => is_graph(character) && !is_alpha(character) && !is_digit(character),
        SPACE => matches!(character, 0x20 | 0x09..=0x0d),
        UPPER => character <= 0x7f && (character as u8).is_ascii_uppercase(),
        XDIGIT => character <= 0x7f && (character as u8).is_ascii_hexdigit(),
        _ => false,
    }
}

macro_rules! export_classifier {
    ($name:ident, $name_l:ident, $kind:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(character: u32) -> c_int {
            classify(character, $kind) as c_int
        }

        #[no_mangle]
        pub extern "C" fn $name_l(character: u32, _locale: *mut c_void) -> c_int {
            classify(character, $kind) as c_int
        }
    };
}

export_classifier!(iswalnum, iswalnum_l, ALNUM);
export_classifier!(iswalpha, iswalpha_l, ALPHA);
export_classifier!(iswblank, iswblank_l, BLANK);
export_classifier!(iswcntrl, iswcntrl_l, CNTRL);
export_classifier!(iswdigit, iswdigit_l, DIGIT);
export_classifier!(iswgraph, iswgraph_l, GRAPH);
export_classifier!(iswlower, iswlower_l, LOWER);
export_classifier!(iswprint, iswprint_l, PRINT);
export_classifier!(iswpunct, iswpunct_l, PUNCT);
export_classifier!(iswspace, iswspace_l, SPACE);
export_classifier!(iswupper, iswupper_l, UPPER);
export_classifier!(iswxdigit, iswxdigit_l, XDIGIT);

#[no_mangle]
pub extern "C" fn iswctype(character: u32, class: u32) -> c_int {
    classify(character, class) as c_int
}

#[no_mangle]
pub extern "C" fn iswctype_l(character: u32, class: u32, _locale: *mut c_void) -> c_int {
    classify(character, class) as c_int
}

fn to_lower(character: u32) -> u32 {
    if character <= 0x7f {
        (character as u8).to_ascii_lowercase() as u32
    } else {
        character
    }
}

fn to_upper(character: u32) -> u32 {
    if character <= 0x7f {
        (character as u8).to_ascii_uppercase() as u32
    } else {
        character
    }
}

#[no_mangle]
pub extern "C" fn towlower(character: u32) -> u32 {
    to_lower(character)
}

#[no_mangle]
pub extern "C" fn towlower_l(character: u32, _locale: *mut c_void) -> u32 {
    to_lower(character)
}

#[no_mangle]
pub extern "C" fn towupper(character: u32) -> u32 {
    to_upper(character)
}

#[no_mangle]
pub extern "C" fn towupper_l(character: u32, _locale: *mut c_void) -> u32 {
    to_upper(character)
}

#[no_mangle]
pub extern "C" fn towctrans(character: u32, transformation: u32) -> u32 {
    match transformation {
        TOLOWER => to_lower(character),
        TOUPPER => to_upper(character),
        _ => character,
    }
}

#[no_mangle]
pub extern "C" fn towctrans_l(
    character: u32,
    transformation: u32,
    _locale: *mut c_void,
) -> u32 {
    towctrans(character, transformation)
}

unsafe fn name_equals(input: *const c_char, expected: &[u8]) -> bool {
    if input.is_null() {
        return false;
    }
    for (index, value) in expected.iter().copied().enumerate() {
        if unsafe { read_volatile(input.cast::<u8>().add(index)) } != value {
            return false;
        }
    }
    unsafe { read_volatile(input.cast::<u8>().add(expected.len())) == 0 }
}

#[no_mangle]
pub unsafe extern "C" fn wctype(name: *const c_char) -> u32 {
    if unsafe { name_equals(name, b"alnum") } { ALNUM }
    else if unsafe { name_equals(name, b"alpha") } { ALPHA }
    else if unsafe { name_equals(name, b"blank") } { BLANK }
    else if unsafe { name_equals(name, b"cntrl") } { CNTRL }
    else if unsafe { name_equals(name, b"digit") } { DIGIT }
    else if unsafe { name_equals(name, b"graph") } { GRAPH }
    else if unsafe { name_equals(name, b"lower") } { LOWER }
    else if unsafe { name_equals(name, b"print") } { PRINT }
    else if unsafe { name_equals(name, b"punct") } { PUNCT }
    else if unsafe { name_equals(name, b"space") } { SPACE }
    else if unsafe { name_equals(name, b"upper") } { UPPER }
    else if unsafe { name_equals(name, b"xdigit") } { XDIGIT }
    else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn wctype_l(name: *const c_char, _locale: *mut c_void) -> u32 {
    unsafe { wctype(name) }
}

#[no_mangle]
pub unsafe extern "C" fn wctrans(name: *const c_char) -> u32 {
    if unsafe { name_equals(name, b"tolower") } { TOLOWER }
    else if unsafe { name_equals(name, b"toupper") } { TOUPPER }
    else { 0 }
}

#[no_mangle]
pub unsafe extern "C" fn wctrans_l(name: *const c_char, _locale: *mut c_void) -> u32 {
    unsafe { wctrans(name) }
}