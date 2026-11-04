#[path = "ffi.rs"]
mod runtime_ffi;
#[path = "libc/mod.rs"]
mod runtime_libc;
#[path = "v8.rs"]
mod runtime_v8;

use crate::kernel::arch::traits::MemoryArchitecture as _;
use crate::kernel::arch::MemoryArchitecture;
use crate::kernel::block::device::BlockDevice;
use crate::kernel::driver::virtio::console::Console;
use crate::kernel::event::r#loop::{EventLoop, EventSourceToken};
use crate::kernel::memory::physical::PhysicalAllocator;
use crate::kernel::net::stack::NetStack;
use crate::KernelContext;

extern "C" {
    fn kz_c_stdio_smoke() -> i32;
    fn kz_c_malloc_smoke() -> i32;
    fn kz_c_mmap_smoke() -> i32;
    fn kz_c_pthread_smoke() -> i32;
}



pub(crate) fn run(context: KernelContext<'_>) {
    let KernelContext {
        events,
        block_device,
        network_stack,
        network_registration,
    } = context;

    if PhysicalAllocator::memory_bytes() > 128 * 1024 * 1024 {
        let _memory_status = Console::write(b"kz: RAM above 128 MiB detected\n");
    }
    let _stdio_status = unsafe { kz_c_stdio_smoke() };
    if unsafe { kz_c_mmap_smoke() } == 0 {
        let _mmap_status = Console::write(b"kz: mmap smoke OK\n");
    }
    if unsafe { kz_c_malloc_smoke() } == 0 {
        let _malloc_status = Console::write(b"kz: malloc/free smoke OK\n");
    }
    if MemoryArchitecture::high_memory_smoke() {
        let _high_memory_status = Console::write(b"kz: RAM above 4 GiB mapped\n");
    }
    if unsafe { kz_c_pthread_smoke() } == 0 {
        let _thread_status = Console::write(b"kz: pthreads smoke OK\n");
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
