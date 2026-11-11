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
#[cfg(feature = "virtualization")]
use crate::arch::traits::VirtualizationArchitecture as _;

pub(crate) mod arch;
pub(crate) mod block;
pub(crate) mod debug;
pub(crate) mod driver;
pub(crate) mod dl;
#[allow(dead_code)]
pub(crate) mod ebpf;
pub(crate) mod error;
pub(crate) mod event;
pub(crate) mod ipc;
#[path = "lib/mod.rs"]
pub(crate) mod kernel_lib;
pub(crate) use kernel_lib as lib;
pub(crate) mod memory;
pub(crate) mod net;
pub(crate) mod namespace;
pub(crate) mod process;
pub(crate) mod syscall;
#[path = "../runtime/run.rs"]
pub(crate) mod runtime;
pub(crate) mod scheduler;
#[cfg(feature = "virtualization")]
pub(crate) mod vm;
#[allow(dead_code)]
pub(crate) mod vfs;

// Preserve existing internal paths while modules move to the crate root.
pub(crate) mod kernel {
    pub(crate) use crate::{arch, block, debug, dl, driver, ebpf, error, event, ipc, lib, memory, net, namespace, process, runtime, scheduler, syscall, vfs};
    #[cfg(feature = "virtualization")]
    pub(crate) use crate::vm;
}

pub(crate) struct KernelContext<'a> {
    pub(crate) events: &'a mut event::r#loop::EventLoop<16>,
    pub(crate) block_device: Option<&'a mut block::service::BlockClient>,
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

    fn main(boot_info: usize, boot_magic: usize) -> ! {
        debug::debug("kz: kernel startup\n");
        let _interrupt_state = arch::InterruptArchitecture::disable_interrupts();
        unsafe {
            arch::InterruptArchitecture::initialize_interrupts();
        }
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
        #[cfg(feature = "virtualization")]
        let _host_virtualization = arch::VirtualizationArchitecture::initialize_host();
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

        let console_ready = driver::virtio::console::Console::initialize();
        let mut virtio_services = driver::virtio::service::KernelVirtioServices::initialize();
        let mut events = event::r#loop::EventLoop::<16>::new();
        let network_registration = events
            .register(
                net::stack::NETWORK_EVENT_SOURCE,
                event::r#loop::EventFlags::READABLE,
                0,
            )
            .ok();
        if let Some(now_ns) = arch::TimerArchitecture::now_ns() {
            if let (Some(stack), Some(source)) =
                (virtio_services.network_stack.as_mut(), network_registration)
            {
                stack.poll(now_ns, &mut events, source);
            }
            let _event = events.wait(now_ns);
        }

        debug::write(arch::BootArchitecture::BOOT_MESSAGE);
        arch::InterruptArchitecture::enable_interrupts();
        if console_ready && driver::virtio::console::Console::write(b"kz: virtio-console transmit OK\n") {
            debug::write_virtio_devices();
            if scheduler::thread::smoke::thread_smoke() {
                debug::debug("kz: kernel thread smoke OK\n");
            } else {
                debug::debug("kz: kernel thread smoke FAILED\n");
            }
            if scheduler::thread::smoke::vm_smoke() {
                debug::debug("kz: user VM smoke OK\n");
            } else {
                debug::debug("kz: user VM smoke FAILED\n");
            }
            if ipc::process_capability_smoke().is_ok()
                && ipc::shared_mapping_smoke().is_ok()
                && ipc::smoke().is_ok()
            {
                debug::debug("kz: IPC smoke OK\n");
            } else {
                debug::debug("kz: IPC smoke FAILED\n");
            }
            if let Some(block_service) = virtio_services.block_device.as_ref() {
                if block_service.smoke().is_ok() {
                    debug::debug("kz: block IPC smoke OK\n");
                } else {
                    debug::debug("kz: block IPC smoke FAILED\n");
                }
            }
            if arch::start_secondary_cpus() > 1 {
                debug::debug("kz: smp secondary cpus online\n");
            }
            runtime::run(KernelContext {
                events: &mut events,
                block_device: virtio_services.block_device.as_mut(),
                network_stack: virtio_services.network_stack.as_mut(),
                network_registration,
            });
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
