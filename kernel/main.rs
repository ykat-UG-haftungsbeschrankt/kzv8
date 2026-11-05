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
    BootArchitecture as _, CpuArchitecture as _, InterruptArchitecture as _, ThreadArchitecture as _,
    TimerArchitecture as _,
};

pub(crate) mod arch;
pub(crate) mod block;
pub(crate) mod driver;
pub(crate) mod error;
pub(crate) mod event;
#[allow(special_module_name)]
pub(crate) mod lib;
pub(crate) mod memory;
pub(crate) mod net;
#[path = "../runtime/run.rs"]
pub(crate) mod runtime;
pub(crate) mod scheduler;
#[allow(dead_code)]
pub(crate) mod vfs;

// Preserve existing internal paths while modules move to the crate root.
pub(crate) mod kernel {
    pub(crate) use crate::{arch, block, driver, error, event, lib, memory, net, runtime, scheduler, vfs};
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
    }

    fn main(boot_info: usize, boot_magic: usize) -> !{
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
        let memory_map_needs_growth = memory::memblock::MemBlock::needs_growth();
        memory::r#virtual::initialize();
        memory::memblock::MemBlock::enable_growth();
        if memory_map_needs_growth {
            memory::memblock::MemBlock::begin_discovery();
            assert!(
                unsafe { arch::BootArchitecture::initialize_boot_info(boot_info, boot_magic) },
                "{}",
                arch::BootArchitecture::BOOT_INFO_ERROR
            );
            assert!(
                memory::memblock::MemBlock::finish_discovery(),
                "unable to map bootstrap memory"
            );
            assert!(
                unsafe { arch::BootArchitecture::initialize_boot_info(boot_info, boot_magic) },
                "{}",
                arch::BootArchitecture::BOOT_INFO_ERROR
            );
            memory::r#virtual::refresh_memory_map();
        }
        assert!(
            memory::physical::PhysicalAllocator::initialize(),
            "unable to initialize physical memory"
        );
        memory::heap::Heap::initialize();
        assert!(
            scheduler::thread::ThreadManager::initialize(),
            "unable to initialize the thread scheduler"
        );
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
        arch::InterruptArchitecture::enable_interrupts();
        if console_ready {
            if driver::virtio::console::Console::write(arch::BootArchitecture::BOOT_MESSAGE)
                && driver::virtio::console::Console::write(b"kz: virtio-console transmit OK\n")
            {
                driver::virtio::console::Console::write_device_list();
                if arch::start_secondary_cpus() > 1 {
                    let _smp_status =
                        driver::virtio::console::Console::write(b"kz: smp secondary cpus online\n");
                }
                runtime::run(KernelContext {
                    events: &mut events,
                    block_device: block_device.as_mut(),
                    network_stack: network_stack.as_mut(),
                    network_registration,
                });
            }
        }
        scheduler::future::run_forever()
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
