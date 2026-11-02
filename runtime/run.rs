#[path = "ffi.rs"]
mod runtime_ffi;
#[path = "libc/mod.rs"]
mod runtime_libc;
#[path = "v8.rs"]
mod runtime_v8;

pub(crate) fn run(
    events: &mut crate::event_loop::EventLoop<16>,
    block_device: Option<&mut crate::block_device::BlockDevice>,
    network_stack: Option<&mut crate::net_stack::NetStack>,
    network_registration: Option<crate::event_loop::EventSourceToken>,
) {
    if crate::runtime::runtime_v8::page_allocator_smoke() {
        let _page_allocator = crate::console::write(b"kz: V8 page allocator hooks OK\n");
    }
    let block_device = block_device
        .map(|device| device as *mut _ as *mut core::ffi::c_void)
        .unwrap_or(core::ptr::null_mut());
    let mut tcp_context =
        runtime_ffi::TcpContext::new(events, network_stack, network_registration);
    match crate::runtime::runtime_v8::run_smoke(
        events,
        block_device,
        &mut tcp_context as *mut _ as *mut core::ffi::c_void,
    ) {
        Some(0) => {}
        Some(_) => {
            let _v8_error = crate::console::write(b"kz: V8 smoke failed\n");
        }
        None => {
            let _v8_status = crate::console::write(b"kz: V8 not integrated\n");
        }
    }
}
