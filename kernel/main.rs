#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};

extern crate alloc;

mod arch;
#[path = "block/device.rs"]
mod block_device;
#[path = "block/request.rs"]
mod block_request;
#[path = "drivers/virtio/console.rs"]
mod console;
#[path = "memory/dma.rs"]
mod dma;
#[path = "event/completion.rs"]
mod event_completion;
#[path = "event/loop.rs"]
mod event_loop;
#[path = "event/timers.rs"]
mod event_timers;
#[path = "memory/heap.rs"]
mod heap;
#[path = "memory/mmap.rs"]
mod kernel_mmap;
#[path = "thread.rs"]
mod kernel_thread;
#[path = "memory/virtual.rs"]
mod memory;
#[path = "net/device.rs"]
mod net_device;
#[path = "net/stack.rs"]
mod net_stack;
#[path = "net/tcp.rs"]
mod net_tcp;
#[path = "platform/pci.rs"]
mod pci;
#[path = "memory/physical.rs"]
mod physical;
#[path = "platform/discovery.rs"]
mod platform;
#[path = "runtime/ffi.rs"]
mod runtime_ffi;
#[path = "runtime/host.rs"]
mod runtime_host;
mod sync;
#[path = "task.rs"]
mod task;
#[path = "drivers/virtio/block.rs"]
mod virtio_block;
#[path = "drivers/virtio/net.rs"]
mod virtio_net;
#[path = "drivers/virtio/queue.rs"]
mod virtio_queue;
#[path = "drivers/virtio/rng.rs"]
mod virtio_rng;
#[path = "drivers/virtio/transport.rs"]
mod virtio_transport;

extern "C" {
    fn kz_c_stdio_smoke() -> i32;
    fn kz_c_malloc_smoke() -> i32;
    fn kz_c_mmap_smoke() -> i32;
    fn kz_c_pthread_smoke() -> i32;
}

static FUTURE_SMOKE_DONE: AtomicBool = AtomicBool::new(false);

#[cfg(target_arch = "x86_64")]
extern "C" {
    fn kz_v8_run_smoke() -> i32;
}

#[no_mangle]
pub extern "C" fn kz_handle_virtio_interrupt() {
    let pending = virtio_transport::dispatch_interrupts();
    event_loop::record_device_interrupts(pending);
    console::handle_interrupt(pending);
}

#[no_mangle]
pub extern "C" fn kz_kernel_main(boot_info: usize, boot_magic: usize) -> ! {
    let _interrupt_state = arch::interrupts::disable();
    unsafe {
        arch::interrupts::initialize();
    }
    #[cfg(target_arch = "x86_64")]
    assert!(
        platform::initialize_x86(boot_info, boot_magic),
        "unable to read the Multiboot memory map"
    );
    #[cfg(target_arch = "aarch64")]
    assert!(
        platform::initialize_aarch64(boot_info),
        "unable to read the device-tree memory map"
    );
    memory::initialize();
    assert!(
        physical::initialize(),
        "unable to initialize physical memory"
    );
    heap::initialize();
    kernel_thread::initialize();
    dma::initialize();
    assert!(
        arch::initialize_interrupt_controller(),
        "unable to initialize the interrupt controller"
    );
    arch::timer::initialize();
    let _block_device = block_device::BlockDevice::initialize();
    let mut network_stack = net_stack::NetStack::initialize();
    let _rng_device = virtio_rng::VirtioRng::initialize();
    let mut events = event_loop::EventLoop::<16>::new();
    let network_registration = events
        .register(
            net_stack::NETWORK_EVENT_SOURCE,
            event_loop::EventFlags::READABLE,
            0,
        )
        .ok();
    if let Some(now_ns) = arch::timer::now_ns() {
        if let (Some(stack), Some(source)) = (network_stack.as_mut(), network_registration) {
            stack.poll(now_ns, &mut events, source);
        }
        let _event = events.wait(now_ns);
    }
    let console_ready = console::initialize();
    let online_cpus = arch::start_secondary_cpus();
    let future_smoke_started = task::spawn(futures_smoke()).is_ok();
    if future_smoke_started {
        task::run_until_stalled();
    }
    arch::interrupts::enable();
    if console_ready {
        #[cfg(target_arch = "x86_64")]
        let message = b"kz: x86_64 Rust kernel booted\n";
        #[cfg(target_arch = "aarch64")]
        let message = b"kz: aarch64 Rust kernel booted\n";
        if console::write(message) && console::write(b"kz: virtio-console transmit OK\n") {
            if online_cpus > 1 {
                let _smp_status = console::write(b"kz: SMP online\n");
            }
            if future_smoke_started && FUTURE_SMOKE_DONE.load(Ordering::Acquire) {
                let _future_status = console::write(b"kz: Rust futures smoke OK\n");
            }
            let _stdio_status = unsafe { kz_c_stdio_smoke() };
            if physical::memory_bytes() > 128 * 1024 * 1024 {
                let _memory_status = console::write(b"kz: RAM above 128 MiB detected\n");
            }
            if unsafe { kz_c_mmap_smoke() } == 0 {
                let _mmap_status = console::write(b"kz: mmap smoke OK\n");
            }
            if unsafe { kz_c_malloc_smoke() } == 0 {
                let _malloc_status = console::write(b"kz: malloc/free smoke OK\n");
            }
            #[cfg(target_arch = "x86_64")]
            if let Some(address) = physical::allocate_contiguous(4096, 4 * 1024 * 1024 * 1024) {
                unsafe { core::ptr::write_volatile(address as *mut u8, 0xa5) };
                let mapped = unsafe { core::ptr::read_volatile(address as *const u8) == 0xa5 };
                let released = physical::deallocate_contiguous(address, 4096);
                if mapped && released {
                    let _high_memory_status = console::write(b"kz: RAM above 4 GiB mapped\n");
                }
            }
            if unsafe { kz_c_pthread_smoke() } == 0 {
                let _thread_status = console::write(b"kz: pthreads smoke OK\n");
            }
            #[cfg(target_arch = "x86_64")]
            if runtime_ffi::v8_page_allocator_smoke() {
                let _page_allocator = console::write(b"kz: V8 page allocator hooks OK\n");
            }
            #[cfg(target_arch = "x86_64")]
            if unsafe { kz_v8_run_smoke() } != 0 {
                let _v8_error = console::write(b"kz: V8 smoke failed\n");
            }
            #[cfg(target_arch = "aarch64")]
            let _v8_status = console::write(b"kz: V8 not integrated\n");
        }
    }
    task::run_forever()
}

async fn futures_smoke() {
    task::yield_now().await;
    FUTURE_SMOKE_DONE.store(true, Ordering::Release);
}

#[no_mangle]
pub extern "C" fn kz_secondary_main(cpu_id: usize) -> ! {
    if !arch::initialize_secondary_cpu(cpu_id) {
        arch::halt();
    }
    arch::mark_cpu_online(cpu_id);
    arch::interrupts::enable();
    task::run_forever()
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    arch::halt()
}
