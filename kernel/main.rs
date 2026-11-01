#![no_std]
#![no_main]

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

#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    let _interrupt_state = arch::interrupts::disable();
    unsafe {
        arch::interrupts::initialize();
    }
    memory::initialize();
    assert!(
        physical::initialize(),
        "unable to initialize physical memory"
    );
    heap::initialize();
    dma::initialize();
    arch::timer::initialize();
    let _block_device = block_device::BlockDevice::initialize();
    let mut network_stack = net_stack::NetStack::initialize();
    let _rng_device = virtio_rng::VirtioRng::initialize();
    let mut events = event_loop::EventLoop::<16>::new();
    if let Some(now_ns) = arch::timer::now_ns() {
        if let Some(stack) = network_stack.as_mut() {
            stack.poll(now_ns);
        }
        events.poll(now_ns);
    }
    if console::initialize() {
        #[cfg(target_arch = "x86_64")]
        let message = b"kz: x86_64 Rust kernel booted\n";
        #[cfg(target_arch = "aarch64")]
        let message = b"kz: aarch64 Rust kernel booted\n";
        if console::write(message) && console::write(b"kz: virtio-console transmit OK\n") {
            let _completed = console::write(b"kz: V8 not integrated\n");
        }
    }
    arch::halt()
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    arch::halt()
}
