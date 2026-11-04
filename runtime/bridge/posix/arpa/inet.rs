use core::ffi::{c_char, c_int, c_void};
use core::ptr::{copy_nonoverlapping, read_volatile};
use super::super::errno::{EAFNOSUPPORT, EFAULT, ENOSPC};

const AF_INET: c_int = 2;
const AF_INET6: c_int = 10;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct InAddr {
    s_addr: u32,
}

static mut INET_NTOA_BUFFER: [u8; 16] = [0; 16];

fn set_errno(error: c_int) {
    unsafe {
        crate::kernel::scheduler::thread::__errno_location().write(error);
    }
}

#[no_mangle]
pub extern "C" fn htonl(value: u32) -> u32 {
    value.to_be()
}

#[no_mangle]
pub extern "C" fn htons(value: u16) -> u16 {
    value.to_be()
}

#[no_mangle]
pub extern "C" fn ntohl(value: u32) -> u32 {
    u32::from_be(value)
}

#[no_mangle]
pub extern "C" fn ntohs(value: u16) -> u16 {
    u16::from_be(value)
}

#[no_mangle]
pub unsafe extern "C" fn inet_addr(text: *const c_char) -> u32 {
    let mut buffer = [0; 16];
    let Some(length) = (unsafe { copy_c_string(text, &mut buffer) }) else {
        return u32::MAX;
    };
    parse_ipv4(&buffer[..length])
        .map(u32::from_ne_bytes)
        .unwrap_or(u32::MAX)
}

#[no_mangle]
pub unsafe extern "C" fn inet_ntoa(address: InAddr) -> *mut c_char {
    let mut formatted = [0; 16];
    let length = format_ipv4(&address.s_addr.to_ne_bytes(), &mut formatted);
    formatted[length] = 0;
    let output = core::ptr::addr_of_mut!(INET_NTOA_BUFFER).cast::<u8>();
    unsafe { copy_nonoverlapping(formatted.as_ptr(), output, length + 1) };
    output.cast()
}

#[no_mangle]
pub unsafe extern "C" fn inet_ntop(
    address_family: c_int,
    source: *const c_void,
    destination: *mut c_char,
    destination_size: u32,
) -> *const c_char {
    if source.is_null() || destination.is_null() {
        set_errno(EFAULT);
        return core::ptr::null();
    }

    let mut formatted = [0; 46];
    let length = match address_family {
        AF_INET => {
            let mut address = [0; 4];
            for (index, byte) in address.iter_mut().enumerate() {
                *byte = unsafe { read_volatile(source.cast::<u8>().add(index)) };
            }
            format_ipv4(&address, &mut formatted)
        }
        AF_INET6 => {
            let mut address = [0; 16];
            for (index, byte) in address.iter_mut().enumerate() {
                *byte = unsafe { read_volatile(source.cast::<u8>().add(index)) };
            }
            format_ipv6(&address, &mut formatted)
        }
        _ => {
            set_errno(EAFNOSUPPORT);
            return core::ptr::null();
        }
    };

    if destination_size as usize <= length {
        set_errno(ENOSPC);
        return core::ptr::null();
    }
    unsafe {
        copy_nonoverlapping(formatted.as_ptr(), destination.cast::<u8>(), length);
        destination.add(length).write(0);
    }
    destination
}

#[no_mangle]
pub unsafe extern "C" fn inet_pton(
    address_family: c_int,
    source: *const c_char,
    destination: *mut c_void,
) -> c_int {
    if source.is_null() || destination.is_null() {
        set_errno(EFAULT);
        return -1;
    }

    match address_family {
        AF_INET => {
            let mut text = [0; 16];
            let Some(length) = (unsafe { copy_c_string(source, &mut text) }) else {
                return 0;
            };
            let Some(address) = parse_ipv4(&text[..length]) else {
                return 0;
            };
            unsafe { copy_nonoverlapping(address.as_ptr(), destination.cast::<u8>(), 4) };
            1
        }
        AF_INET6 => {
            let mut text = [0; 46];
            let Some(length) = (unsafe { copy_c_string(source, &mut text) }) else {
                return 0;
            };
            let Some(address) = parse_ipv6(&text[..length]) else {
                return 0;
            };
            for (index, group) in address.iter().enumerate() {
                let bytes = group.to_be_bytes();
                unsafe {
                    destination.cast::<u8>().add(index * 2).write(bytes[0]);
                    destination.cast::<u8>().add(index * 2 + 1).write(bytes[1]);
                }
            }
            1
        }
        _ => {
            set_errno(EAFNOSUPPORT);
            -1
        }
    }
}

unsafe fn copy_c_string(source: *const c_char, output: &mut [u8]) -> Option<usize> {
    if source.is_null() {
        return None;
    }
    for (index, byte) in output.iter_mut().enumerate() {
        let value = unsafe { read_volatile(source.add(index).cast::<u8>()) };
        if value == 0 {
            return Some(index);
        }
        *byte = value;
    }
    None
}

fn parse_ipv4(text: &[u8]) -> Option<[u8; 4]> {
    let mut address = [0; 4];
    let mut octet = 0u16;
    let mut octet_index = 0;
    let mut digits = 0;

    for byte in text.iter().copied() {
        match byte {
            b'0'..=b'9' => {
                octet = octet.checked_mul(10)?.checked_add((byte - b'0') as u16)?;
                if octet > 255 {
                    return None;
                }
                digits += 1;
            }
            b'.' if digits != 0 && octet_index < 3 => {
                address[octet_index] = octet as u8;
                octet_index += 1;
                octet = 0;
                digits = 0;
            }
            _ => return None,
        }
    }

    if digits == 0 || octet_index != 3 {
        return None;
    }
    address[3] = octet as u8;
    Some(address)
}

fn parse_ipv6(text: &[u8]) -> Option<[u16; 8]> {
    let mut address = [0; 8];
    let mut compression = None;
    for index in 0..text.len().saturating_sub(1) {
        if text[index] == b':' && text[index + 1] == b':' {
            if compression.replace(index).is_some() {
                return None;
            }
        }
    }

    if let Some(compression_index) = compression {
        let left_count = parse_ipv6_side(&text[..compression_index], false, &mut address)?;
        let mut right = [0; 8];
        let right_count = parse_ipv6_side(&text[compression_index + 2..], true, &mut right)?;
        if left_count + right_count >= 8 {
            return None;
        }
        let right_start = 8 - right_count;
        address[right_start..].copy_from_slice(&right[..right_count]);
    } else if parse_ipv6_side(text, true, &mut address)? != 8 {
        return None;
    }
    Some(address)
}

fn parse_ipv6_side(text: &[u8], allow_ipv4: bool, output: &mut [u16; 8]) -> Option<usize> {
    if text.is_empty() {
        return Some(0);
    }

    let mut group_count = 0;
    let mut start = 0;
    while start < text.len() {
        let end = text[start..]
            .iter()
            .position(|byte| *byte == b':')
            .map(|offset| start + offset)
            .unwrap_or(text.len());
        let part = &text[start..end];
        if part.is_empty() {
            return None;
        }

        if part.contains(&b'.') {
            if !allow_ipv4 || end != text.len() || group_count > 6 {
                return None;
            }
            let octets = parse_ipv4(part)?;
            output[group_count] = u16::from_be_bytes([octets[0], octets[1]]);
            output[group_count + 1] = u16::from_be_bytes([octets[2], octets[3]]);
            group_count += 2;
        } else {
            if group_count == 8 {
                return None;
            }
            output[group_count] = parse_hex_group(part)?;
            group_count += 1;
        }

        if end == text.len() {
            break;
        }
        start = end + 1;
        if start == text.len() {
            return None;
        }
    }
    Some(group_count)
}

fn parse_hex_group(text: &[u8]) -> Option<u16> {
    if text.is_empty() || text.len() > 4 {
        return None;
    }
    let mut value = 0u16;
    for byte in text.iter().copied() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        value = (value << 4) | digit as u16;
    }
    Some(value)
}

fn format_ipv4(address: &[u8; 4], output: &mut [u8]) -> usize {
    let mut length = 0;
    for (index, octet) in address.iter().copied().enumerate() {
        if index != 0 {
            output[length] = b'.';
            length += 1;
        }
        length += format_decimal(octet, &mut output[length..]);
    }
    length
}

fn format_decimal(value: u8, output: &mut [u8]) -> usize {
    if value >= 100 {
        output[0] = b'0' + value / 100;
        output[1] = b'0' + value / 10 % 10;
        output[2] = b'0' + value % 10;
        3
    } else if value >= 10 {
        output[0] = b'0' + value / 10;
        output[1] = b'0' + value % 10;
        2
    } else {
        output[0] = b'0' + value;
        1
    }
}

fn format_ipv6(address: &[u8; 16], output: &mut [u8]) -> usize {
    let mut groups = [0; 8];
    for (index, group) in groups.iter_mut().enumerate() {
        *group = u16::from_be_bytes([address[index * 2], address[index * 2 + 1]]);
    }

    let mut best_start = 8;
    let mut best_length = 1;
    let mut index = 0;
    while index < 8 {
        if groups[index] != 0 {
            index += 1;
            continue;
        }
        let start = index;
        while index < 8 && groups[index] == 0 {
            index += 1;
        }
        let length = index - start;
        if length > best_length {
            best_start = start;
            best_length = length;
        }
    }

    let mut output_length = 0;
    let mut group_index = 0;
    while group_index < 8 {
        if group_index == best_start {
            output[output_length] = b':';
            output[output_length + 1] = b':';
            output_length += 2;
            group_index += best_length;
            if group_index == 8 {
                break;
            }
        } else {
            if output_length != 0 && output[output_length - 1] != b':' {
                output[output_length] = b':';
                output_length += 1;
            }
            output_length += format_hex_group(groups[group_index], &mut output[output_length..]);
            group_index += 1;
        }
    }
    output_length
}

fn format_hex_group(value: u16, output: &mut [u8]) -> usize {
    let mut started = false;
    let mut length = 0;
    for shift in (0..=3).rev() {
        let digit = ((value >> (shift * 4)) & 0x0f) as u8;
        if digit != 0 || started || shift == 0 {
            output[length] = if digit < 10 {
                b'0' + digit
            } else {
                b'a' + digit - 10
            };
            length += 1;
            started = true;
        }
    }
    length
}