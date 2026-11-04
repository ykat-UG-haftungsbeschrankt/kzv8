/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#![no_std]
#![no_main]

extern crate alloc;

use crate::arch::traits::{
    BootArchitecture as _, CpuArchitecture as _, InterruptArchitecture as _,
    ThreadArchitecture as _, TimerArchitecture as _,
};

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
pub(crate) mod vfs;

// Preserve existing internal paths while modules move to the crate root.
pub(crate) mod kernel {
    pub(crate) use crate::{
        arch, block, driver, error, event, memory, net, runtime, scheduler, vfs,
    };
}

pub(crate) struct KernelContext<'a> {
    pub(crate) events: &'a mut event::r#loop::EventLoop<16>,
    pub(crate) block_device: Option<&'a mut block::device::BlockDevice>,
    pub(crate) network_stack: Option<&'a mut net::stack::NetStack>,
    pub(crate) network_registration: Option<event::r#loop::EventSourceToken>,
}

struct Kernel;

impl Kernel {
    fn interrupt() {
        let pending = driver::virtio::transport::dispatch_interrupts();
        event::r#loop::EventLoop::<16>::record_device_interrupts(pending);
        driver::virtio::console::Console::handle_interrupt(pending);
        driver::virtio::input::VirtioInput::handle_interrupt(pending);
    }

    fn main(boot_info: usize, boot_magic: usize) -> ! {
        let _interrupt_state = arch::InterruptArchitecture::disable_interrupts();
        unsafe {
            arch::InterruptArchitecture::initialize_interrupts();
        }
        // SAFETY: the bootloader supplies the architecture-specific boot-info pointer.
        assert!(
            unsafe { arch::BootArchitecture::initialize_boot_info(boot_info, boot_magic) },
            "{}",
            arch::BootArchitecture::BOOT_INFO_ERROR
        );
        memory::r#virtual::initialize();
        assert!(
            memory::physical::PhysicalAllocator::initialize(),
            "unable to initialize physical memory"
        );
        memory::heap::Heap::initialize();
        scheduler::thread::ThreadManager::initialize();
        assert!(
            arch::ThreadArchitecture::initialize_thread_local_storage(0),
            "unable to initialize thread-local storage"
        );
        memory::dma::DmaArena::initialize();
        assert!(
            arch::InterruptArchitecture::initialize_interrupt_controller(),
            "unable to initialize the interrupt controller"
        );
        arch::TimerArchitecture::initialize_timer();
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
        if let Some(now_ns) = arch::TimerArchitecture::now_ns() {
            if let (Some(stack), Some(source)) = (network_stack.as_mut(), network_registration) {
                stack.poll(now_ns, &mut events, source);
            }
            let _event = events.wait(now_ns);
        }
        let console_ready = driver::virtio::console::Console::initialize();
        let _input_ready = driver::virtio::input::VirtioInput::initialize();
        let _vga_device = driver::virtio::vga::VirtioGpu::initialize();
        let _scsi_device = driver::virtio::scsi::VirtioScsi::initialize();
        let _rtc_device = driver::virtio::rtc::VirtioRtc::initialize();
        let virtio_fs = driver::virtio::fs::VirtioFs::initialize();
        vfs::mount_virtio_devices(virtio_fs.as_ref().map(|filesystem| filesystem.root()));
        arch::InterruptArchitecture::enable_interrupts();
        if console_ready {
            if driver::virtio::console::Console::write(arch::BootArchitecture::BOOT_MESSAGE)
                && driver::virtio::console::Console::write(b"kz: virtio-console transmit OK\n")
            {
                driver::virtio::console::Console::write_device_list();
                runtime::run(KernelContext {
                    events: &mut events,
                    block_device: block_device.as_mut(),
                    network_stack: network_stack.as_mut(),
                    network_registration,
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
