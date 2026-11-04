#[repr(C)]
pub struct In6Addr {
    pub s6_addr: [u8; 16],
}

#[allow(non_upper_case_globals)]
#[no_mangle]
pub static in6addr_any: In6Addr = In6Addr { s6_addr: [0; 16] };

#[allow(non_upper_case_globals)]
#[no_mangle]
pub static in6addr_loopback: In6Addr = In6Addr {
    s6_addr: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
};