use core::ffi::{c_char, c_int};
use core::ptr::read_volatile;

const FNM_NOMATCH: c_int = 1;
const FNM_PATHNAME: c_int = 0x01;
const FNM_NOESCAPE: c_int = 0x02;
const FNM_PERIOD: c_int = 0x04;
const FNM_CASEFOLD: c_int = 0x08;

unsafe fn byte(pointer: *const u8) -> u8 {
    unsafe { read_volatile(pointer) }
}

fn fold(value: u8, flags: c_int) -> u8 {
    if flags & FNM_CASEFOLD != 0 {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

unsafe fn component_start(string: *const u8, base: *const u8, flags: c_int) -> bool {
    string == base
        || (flags & FNM_PATHNAME != 0
            && unsafe { byte(string.sub(1)) } == b'/')
}

unsafe fn class_character(pattern: *mut *const u8, flags: c_int) -> Option<u8> {
    let mut value = unsafe { byte(*pattern) };
    if value == 0 {
        return None;
    }
    if value == b'\\' && flags & FNM_NOESCAPE == 0 {
        let next = unsafe { byte((*pattern).add(1)) };
        if next != 0 {
            *pattern = unsafe { (*pattern).add(1) };
            value = next;
        }
    }
    *pattern = unsafe { (*pattern).add(1) };
    Some(value)
}

unsafe fn named_class(pattern: *const u8, character: u8, flags: c_int) -> Option<(bool, *const u8)> {
    if unsafe { byte(pattern) } != b'[' || unsafe { byte(pattern.add(1)) } != b':' {
        return None;
    }
    let mut name = [0; 8];
    let mut name_length = 0;
    let mut cursor = unsafe { pattern.add(2) };
    while unsafe { byte(cursor) } != 0
        && !(unsafe { byte(cursor) } == b':' && unsafe { byte(cursor.add(1)) } == b']')
    {
        if name_length == name.len() {
            return None;
        }
        name[name_length] = unsafe { byte(cursor) };
        name_length += 1;
        cursor = unsafe { cursor.add(1) };
    }
    if unsafe { byte(cursor) } != b':' || unsafe { byte(cursor.add(1)) } != b']' {
        return None;
    }

    let value = if flags & FNM_CASEFOLD != 0 {
        character.to_ascii_lowercase()
    } else {
        character
    };
    let alnum = value.is_ascii_alphanumeric();
    let alpha = value.is_ascii_alphabetic();
    let digit = value.is_ascii_digit();
    let graph = (0x21..=0x7e).contains(&value);
    let class_match = match &name[..name_length] {
        b"alnum" => alnum,
        b"alpha" => alpha,
        b"blank" => matches!(value, b' ' | b'\t'),
        b"cntrl" => value < 0x20 || value == 0x7f,
        b"digit" => digit,
        b"graph" => graph,
        b"lower" => {
            if flags & FNM_CASEFOLD != 0 {
                alpha
            } else {
                value.is_ascii_lowercase()
            }
        }
        b"print" => (0x20..=0x7e).contains(&value),
        b"punct" => graph && !alnum,
        b"space" => value.is_ascii_whitespace(),
        b"upper" => {
            if flags & FNM_CASEFOLD != 0 {
                alpha
            } else {
                value.is_ascii_uppercase()
            }
        }
        b"xdigit" => value.is_ascii_hexdigit(),
        _ => return None,
    };
    Some((class_match, unsafe { cursor.add(2) }))
}

unsafe fn bracket_match(
    pattern: *const u8,
    character: u8,
    flags: c_int,
) -> Option<(bool, *const u8)> {
    let mut cursor = unsafe { pattern.add(1) };
    let mut negate = false;
    if matches!(unsafe { byte(cursor) }, b'!' | b'^') {
        negate = true;
        cursor = unsafe { cursor.add(1) };
    }

    let mut matched = false;
    let mut saw_character = false;
    let mut first = true;
    loop {
        let current = unsafe { byte(cursor) };
        if current == 0 || (current == b']' && !first) {
            break;
        }
        if current == b'[' {
            if let Some((class_match, next)) = unsafe { named_class(cursor, character, flags) } {
                matched |= class_match;
                saw_character = true;
                first = false;
                cursor = next;
                continue;
            }
        }
        let mut position = cursor;
        let Some(start) = (unsafe { class_character(&mut position, flags) }) else {
            return None;
        };
        first = false;
        saw_character = true;

        if unsafe { byte(position) } == b'-'
            && unsafe { byte(position.add(1)) } != 0
            && unsafe { byte(position.add(1)) } != b']'
        {
            let mut end_position = unsafe { position.add(1) };
            if let Some(end) = unsafe { class_character(&mut end_position, flags) } {
                let value = fold(character, flags);
                let start = fold(start, flags);
                let end = fold(end, flags);
                if start <= end && (start..=end).contains(&value) {
                    matched = true;
                }
                cursor = end_position;
                continue;
            }
        }

        if fold(start, flags) == fold(character, flags) {
            matched = true;
        }
        cursor = position;
    }

    if unsafe { byte(cursor) } != b']' || !saw_character {
        return None;
    }
    Some((matched != negate, unsafe { cursor.add(1) }))
}

unsafe fn match_from(
    pattern: *const u8,
    string: *const u8,
    base: *const u8,
    flags: c_int,
) -> bool {
    let pattern_byte = unsafe { byte(pattern) };
    let string_byte = unsafe { byte(string) };

    match pattern_byte {
        0 => string_byte == 0,
        b'*' => {
            let mut next_pattern = unsafe { pattern.add(1) };
            while unsafe { byte(next_pattern) } == b'*' {
                next_pattern = unsafe { next_pattern.add(1) };
            }
            let mut candidate = string;
            loop {
                if unsafe { match_from(next_pattern, candidate, base, flags) } {
                    return true;
                }
                let current = unsafe { byte(candidate) };
                if current == 0
                    || (flags & FNM_PATHNAME != 0 && current == b'/')
                    || (flags & FNM_PERIOD != 0
                        && current == b'.'
                        && unsafe { component_start(candidate, base, flags) })
                {
                    return false;
                }
                candidate = unsafe { candidate.add(1) };
            }
        }
        b'?' => {
            string_byte != 0
                && !(flags & FNM_PATHNAME != 0 && string_byte == b'/')
                && !(flags & FNM_PERIOD != 0
                    && string_byte == b'.'
                    && unsafe { component_start(string, base, flags) })
                && unsafe { match_from(pattern.add(1), string.add(1), base, flags) }
        }
        b'[' => {
            if string_byte == 0
                || (flags & FNM_PATHNAME != 0 && string_byte == b'/')
                || (flags & FNM_PERIOD != 0
                    && string_byte == b'.'
                    && unsafe { component_start(string, base, flags) })
            {
                return false;
            }
            if let Some((matched, next_pattern)) =
                unsafe { bracket_match(pattern, string_byte, flags) }
            {
                matched && unsafe { match_from(next_pattern, string.add(1), base, flags) }
            } else {
                string_byte == b'['
                    && unsafe { match_from(pattern.add(1), string.add(1), base, flags) }
            }
        }
        b'\\' if flags & FNM_NOESCAPE == 0 => {
            let next_pattern = unsafe { byte(pattern.add(1)) };
            if next_pattern == 0 {
                string_byte == b'\\'
                    && unsafe { match_from(pattern.add(1), string.add(1), base, flags) }
            } else {
                fold(next_pattern, flags) == fold(string_byte, flags)
                    && unsafe { match_from(pattern.add(2), string.add(1), base, flags) }
            }
        }
        literal => {
            fold(literal, flags) == fold(string_byte, flags)
                && unsafe { match_from(pattern.add(1), string.add(1), base, flags) }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn fnmatch(
    pattern: *const c_char,
    string: *const c_char,
    flags: c_int,
) -> c_int {
    if pattern.is_null() || string.is_null() {
        return FNM_NOMATCH;
    }
    if unsafe {
        match_from(
            pattern.cast(),
            string.cast(),
            string.cast(),
            flags,
        )
    } {
        0
    } else {
        FNM_NOMATCH
    }
}