/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

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
mod scheduler;
pub(crate) use scheduler::sync;
pub(crate) use scheduler::thread as kernel_thread;
#[path = "memory/virtual.rs"]
mod memory;
#[path = "net/device.rs"]
mod net_device;
#[path = "net/stack.rs"]
mod net_stack;
#[path = "net/tcp.rs"]
mod net_tcp;
#[path = "memory/physical.rs"]
mod physical;
#[path = "memory/resources.rs"]
mod memory_resources;
#[path = "../runtime/run.rs"]
mod runtime;
#[path = "drivers/virtio/block.rs"]
mod virtio_block;
#[path = "drivers/virtio/net.rs"]
mod virtio_net;
#[path = "drivers/virtio/queue.rs"]
mod virtio_queue;
#[path = "drivers/virtio/transport.rs"]
mod virtio_transport;

static FUTURE_SMOKE_DONE: AtomicBool = AtomicBool::new(false);

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
    // SAFETY: the bootloader supplies the architecture-specific boot-info pointer.
    assert!(
        unsafe { arch::initialize_boot_info(boot_info, boot_magic) },
        "{}",
        arch::BOOT_INFO_ERROR
    );
    memory::initialize();
    assert!(
        physical::initialize(),
        "unable to initialize physical memory"
    );
    heap::initialize();
    kernel_thread::initialize();
    assert!(
        arch::initialize_thread_local_storage(0),
        "unable to initialize thread-local storage"
    );
    dma::initialize();
    assert!(
        arch::initialize_interrupt_controller(),
        "unable to initialize the interrupt controller"
    );
    arch::timer::initialize();
    let mut block_device = block_device::BlockDevice::initialize();
    let mut network_stack = net_stack::NetStack::initialize();
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
    let future_smoke_started = scheduler::task::spawn(futures_smoke()).is_ok();
    if future_smoke_started {
        scheduler::task::run_until_stalled();
    }
    arch::interrupts::enable();
    if console_ready {
        if console::write(arch::BOOT_MESSAGE)
        && console::write(b"kz: virtio-console transmit OK\n") {
            runtime::run(
                &mut events,
                block_device.as_mut(),
                network_stack.as_mut(),
                network_registration,
            );
        }
    }
    scheduler::task::run_forever()
}

async fn futures_smoke() {
    scheduler::task::yield_now().await;
    FUTURE_SMOKE_DONE.store(true, Ordering::Release);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    arch::halt()
}
