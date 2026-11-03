#[path = "ffi.rs"]
mod runtime_ffi;
#[path = "libc/mod.rs"]
mod runtime_libc;
#[path = "v8.rs"]
mod runtime_v8;

use crate::kernel::arch;
use crate::kernel::block::device::BlockDevice;
use crate::kernel::drivers::virtio::console::Console;
use crate::kernel::event::r#loop::{EventLoop, EventSourceToken};
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::net::stack::NetStack;

extern "C" {
    fn kz_c_stdio_smoke() -> i32;
    fn kz_c_malloc_smoke() -> i32;
    fn kz_c_mmap_smoke() -> i32;
    fn kz_c_free_smoke() -> i32;
    fn kz_c_pthread_smoke() -> i32;
}

pub(crate) fn run(
    events: &mut EventLoop<16>,
    block_device: Option<&mut BlockDevice>,
    network_stack: Option<&mut NetStack>,
    network_registration: Option<EventSourceToken>,
    future_smoke_ok: bool,
) {
    if future_smoke_ok {
        let _future_status = Console::write(b"kz: Rust futures smoke OK\n");
    }
    if PhysicalAllocator::memory_bytes() > 128 * 1024 * 1024 {
        let _memory_status = Console::write(b"kz: RAM above 128 MiB detected\n");
    }
    let _stdio_status = unsafe { kz_c_stdio_smoke() };
    if unsafe { kz_c_mmap_smoke() } == 0 {
        let _mmap_status = Console::write(b"kz: mmap smoke OK\n");
    }
    if unsafe { kz_c_malloc_smoke() } == 0 {
        let _malloc_status = Console::write(b"kz: malloc smoke OK\n");
    }
    if unsafe { kz_c_free_smoke() } == 0 {
        let _free_status = Console::write(b"kz: free smoke OK\n");
    }
    if arch::high_memory_smoke() {
        let _high_memory_status = Console::write(b"kz: RAM above 4 GiB mapped\n");
    }
    if unsafe { kz_c_pthread_smoke() } == 0 {
        let _thread_status = Console::write(b"kz: pthreads smoke OK\n");
    }

    if runtime_v8::page_allocator_smoke() {
        let _page_allocator = Console::write(b"kz: V8 page allocator hooks OK\n");
    }
    let block_device = block_device
        .map(|device| device as *mut _ as *mut core::ffi::c_void)
        .unwrap_or(core::ptr::null_mut());
    let mut tcp_context = runtime_ffi::TcpContext::new(events, network_stack, network_registration);
    match runtime_v8::run_smoke(
        events,
        block_device,
        &mut tcp_context as *mut _ as *mut core::ffi::c_void,
    ) {
        Some(0) => {}
        Some(_) => {
            let _v8_error = Console::write(b"kz: V8 smoke failed\n");
        }
        None => {
            let _v8_status = Console::write(b"kz: V8 not integrated\n");
        }
    }
}
