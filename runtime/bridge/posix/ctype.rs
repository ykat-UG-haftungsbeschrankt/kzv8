use core::ffi::{c_int, c_void};

fn byte(character: c_int) -> Option<u8> {
    (0..=u8::MAX as c_int)
        .contains(&character)
        .then_some(character as u8)
}

fn is_alpha(character: c_int) -> bool {
    byte(character).is_some_and(|value| value.is_ascii_alphabetic())
}

fn is_digit(character: c_int) -> bool {
    byte(character).is_some_and(|value| value.is_ascii_digit())
}

fn is_alnum(character: c_int) -> bool {
    is_alpha(character) || is_digit(character)
}

fn is_blank(character: c_int) -> bool {
    matches!(character, 0x20 | 0x09)
}

fn is_cntrl(character: c_int) -> bool {
    (0..=0x1f).contains(&character) || character == 0x7f
}

fn is_graph(character: c_int) -> bool {
    (0x21..=0x7e).contains(&character)
}

fn is_lower(character: c_int) -> bool {
    byte(character).is_some_and(|value| value.is_ascii_lowercase())
}

fn is_print(character: c_int) -> bool {
    (0x20..=0x7e).contains(&character)
}

fn is_punct(character: c_int) -> bool {
    is_graph(character) && !is_alnum(character)
}

fn is_space(character: c_int) -> bool {
    matches!(character, 0x20 | 0x09..=0x0d)
}

fn is_upper(character: c_int) -> bool {
    byte(character).is_some_and(|value| value.is_ascii_uppercase())
}

fn is_xdigit(character: c_int) -> bool {
    byte(character).is_some_and(|value| value.is_ascii_hexdigit())
}

macro_rules! export_classifier {
    ($name:ident, $locale_name:ident, $predicate:ident) => {
        #[no_mangle]
        pub extern "C" fn $name(character: c_int) -> c_int {
            $predicate(character) as c_int
        }

        #[no_mangle]
        pub extern "C" fn $locale_name(character: c_int, _locale: *mut c_void) -> c_int {
            $predicate(character) as c_int
        }
    };
}

export_classifier!(isalnum, isalnum_l, is_alnum);
export_classifier!(isalpha, isalpha_l, is_alpha);
export_classifier!(isblank, isblank_l, is_blank);
export_classifier!(iscntrl, iscntrl_l, is_cntrl);
export_classifier!(isdigit, isdigit_l, is_digit);
export_classifier!(isgraph, isgraph_l, is_graph);
export_classifier!(islower, islower_l, is_lower);
export_classifier!(isprint, isprint_l, is_print);
export_classifier!(ispunct, ispunct_l, is_punct);
export_classifier!(isspace, isspace_l, is_space);
export_classifier!(isupper, isupper_l, is_upper);
export_classifier!(isxdigit, isxdigit_l, is_xdigit);

fn to_lower(character: c_int) -> c_int {
    byte(character)
        .map(|value| value.to_ascii_lowercase() as c_int)
        .unwrap_or(character)
}

fn to_upper(character: c_int) -> c_int {
    byte(character)
        .map(|value| value.to_ascii_uppercase() as c_int)
        .unwrap_or(character)
}

#[no_mangle]
pub extern "C" fn tolower(character: c_int) -> c_int {
    to_lower(character)
}

#[no_mangle]
pub extern "C" fn tolower_l(character: c_int, _locale: *mut c_void) -> c_int {
    to_lower(character)
}

#[no_mangle]
pub extern "C" fn toupper(character: c_int) -> c_int {
    to_upper(character)
}

#[no_mangle]
pub extern "C" fn toupper_l(character: c_int, _locale: *mut c_void) -> c_int {
    to_upper(character)
}