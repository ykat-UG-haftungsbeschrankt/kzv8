/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#![no_std]
#![no_main]

extern crate alloc;

pub(crate) mod arch;
pub(crate) mod block;
pub(crate) mod driver;
pub(crate) mod error;
pub(crate) mod event;
pub(crate) mod memory;
pub(crate) mod net;
#[path = "../runtime/run.rs"]
pub(crate) mod runtime;
pub(crate) mod scheduler;

#[cfg(feature = "kernel-smoke-tests")]
#[path = "../tests/kernel/futures.rs"]
mod future_smoke;

// Preserve existing internal paths while modules move to the crate root.
pub(crate) mod kernel {
    pub(crate) use crate::{arch, block, driver, error, event, memory, net, runtime, scheduler};
}

pub(crate) struct KernelContext<'a> {
    pub(crate) events: &'a mut EventLoop<16>,
    pub(crate) block_device: Option<&'a mut BlockDevice>,
    pub(crate) network_stack: Option<&'a mut NetStack>,
    pub(crate) network_registration: Option<EventSourceToken>,
    pub(crate) future_smoke_ok: bool,
}

impl Kernel {
    type Output = ();

    fn interrupt() {
        let pending = driver::virtio::transport::dispatch_interrupts();
        event::r#loop::EventLoop::<16>::record_device_interrupts(pending);
        driver::virtio::console::Console::handle_interrupt(pending);
    }

    fn main(boot_info: usize, boot_magic: usize) -> !{
        let _interrupt_state =
            <arch::MemoryArchitecture as arch::InterruptArchitecture>::disable_interrupts();
        unsafe {
            <arch::MemoryArchitecture as arch::InterruptArchitecture>::initialize_interrupts();
        }
        // SAFETY: the bootloader supplies the architecture-specific boot-info pointer.
        assert!(
            unsafe {
                <arch::MemoryArchitecture as arch::BootArchitecture>::initialize_boot_info(
                    boot_info, boot_magic,
                )
            },
            "{}",
            <arch::MemoryArchitecture as arch::BootArchitecture>::BOOT_INFO_ERROR
        );
        memory::r#virtual::initialize();
        assert!(
            memory::physical::PhysicalAllocator::initialize(),
            "unable to initialize physical memory"
        );
        memory::heap::Heap::initialize();
        scheduler::thread::ThreadManager::initialize();
        assert!(
            <arch::MemoryArchitecture as arch::ThreadArchitecture>::initialize_thread_local_storage(0),
            "unable to initialize thread-local storage"
        );
        memory::dma::DmaArena::initialize();
        assert!(
            <arch::MemoryArchitecture as arch::InterruptArchitecture>::initialize_interrupt_controller(),
            "unable to initialize the interrupt controller"
        );
        <arch::MemoryArchitecture as arch::TimerArchitecture>::initialize_timer();
        let mut block_device = block::device::BlockDevice::initialize();
        let mut network_stack = net::stack::NetStack::initialize();
        let mut events = event::r#loop::EventLoop::<16>::new();
        let network_registration = events
            .register(
                net::stack::NETWORK_EVENT_SOURCE,
                event::r#loop::EventFlags::READABLE,
                0,
            )
            .ok();
        if let Some(now_ns) = arch::timer::now_ns() {
            if let (Some(stack), Some(source)) = (network_stack.as_mut(), network_registration) {
                stack.poll(now_ns, &mut events, source);
            }
            let _event = events.wait(now_ns);
        }
        let console_ready = driver::virtio::console::Console::initialize();
        #[cfg(feature = "kernel-smoke-tests")]
        let future_smoke_ok = future_smoke::run();
        #[cfg(not(feature = "kernel-smoke-tests"))]
        let future_smoke_ok = false;
        <arch::MemoryArchitecture as arch::InterruptArchitecture>::enable_interrupts();
        if console_ready {
            if driver::virtio::console::Console::write(arch::BOOT_MESSAGE)
                && driver::virtio::console::Console::write(b"kz: virtio-console transmit OK\n")
            {
                runtime::run(runtime::KernelContext {
                    events: &mut events,
                    block_device: block_device.as_mut(),
                    network_stack: network_stack.as_mut(),
                    network_registration,
                    future_smoke_ok,
                });
            }
        }
        scheduler::task::run_forever()
    }

    fn halt() -> ! {
        arch::CpuArchitecture::halt()
    }
}

#[no_mangle]
pub extern "C" fn kz_handle_virtio_interrupt() {
    Kernel::interrupt();
}

#[no_mangle]
pub extern "C" fn kz_kernel_main(boot_info: usize, boot_magic: usize) -> ! {
    Kernel::main(boot_info, boot_magic);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    Kernel::halt()
}
