#[path = "ffi.rs"]
mod runtime_ffi;
#[path = "libc/mod.rs"]
mod runtime_libc;
#[path = "v8.rs"]
mod runtime_v8;

extern "C" {
    fn kz_c_stdio_smoke() -> i32;
    fn kz_c_malloc_smoke() -> i32;
    fn kz_c_mmap_smoke() -> i32;
    fn kz_c_free_smoke() -> i32;
    fn kz_c_pthread_smoke() -> i32;
}

pub(crate) fn run(
    events: &mut crate::event_loop::EventLoop<16>,
    block_device: Option<&mut crate::block_device::BlockDevice>,
    network_stack: Option<&mut crate::net_stack::NetStack>,
    network_registration: Option<crate::event_loop::EventSourceToken>,
) {
    if future_smoke_started && FUTURE_SMOKE_DONE.load(Ordering::Acquire) {
        let _future_status = console::write(b"kz: Rust futures smoke OK\n");
    }
    if physical::memory_bytes() > 128 * 1024 * 1024 {
        let _memory_status = console::write(b"kz: RAM above 128 MiB detected\n");
    }
    let _stdio_status = unsafe { kz_c_stdio_smoke() };
    if unsafe { kz_c_mmap_smoke() } == 0 {
        let _mmap_status = console::write(b"kz: mmap smoke OK\n");
    }
    if unsafe { kz_c_malloc_smoke() } == 0 {
        let _malloc_status = console::write(b"kz: malloc smoke OK\n");
    }
    if unsafe { kz_c_free_smoke() } == 0 {
        let _free_status = console::write(b"kz: free smoke OK\n");
    }
    if arch::high_memory_smoke() {
        let _high_memory_status = console::write(b"kz: RAM above 4 GiB mapped\n");
    }
    if unsafe { kz_c_pthread_smoke() } == 0 {
        let _thread_status = console::write(b"kz: pthreads smoke OK\n");
    }


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
