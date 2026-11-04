#[no_mangle]
pub extern "C" fn be16toh(value: u16) -> u16 {
    u16::from_be(value)
}

#[no_mangle]
pub extern "C" fn be32toh(value: u32) -> u32 {
    u32::from_be(value)
}

#[no_mangle]
pub extern "C" fn be64toh(value: u64) -> u64 {
    u64::from_be(value)
}

#[no_mangle]
pub extern "C" fn htobe16(value: u16) -> u16 {
    value.to_be()
}

#[no_mangle]
pub extern "C" fn htobe32(value: u32) -> u32 {
    value.to_be()
}

#[no_mangle]
pub extern "C" fn htobe64(value: u64) -> u64 {
    value.to_be()
}

#[no_mangle]
pub extern "C" fn htole16(value: u16) -> u16 {
    value.to_le()
}

#[no_mangle]
pub extern "C" fn htole32(value: u32) -> u32 {
    value.to_le()
}

#[no_mangle]
pub extern "C" fn htole64(value: u64) -> u64 {
    value.to_le()
}

#[no_mangle]
pub extern "C" fn le16toh(value: u16) -> u16 {
    u16::from_le(value)
}

#[no_mangle]
pub extern "C" fn le32toh(value: u32) -> u32 {
    u32::from_le(value)
}

#[no_mangle]
pub extern "C" fn le64toh(value: u64) -> u64 {
    u64::from_le(value)
}