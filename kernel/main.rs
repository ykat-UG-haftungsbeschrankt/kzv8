/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

#[cfg(all(test, target_arch = "x86_64"))]
#[export_name = "pml4"]
static mut TEST_PML4: u64 = 0;

#[cfg(all(test, target_arch = "x86_64"))]
#[export_name = "__bss_end"]
static mut TEST_BSS_END: u8 = 0;

use crate::arch::traits::{
    BootArchitecture as _, CpuArchitecture as _, InterruptArchitecture as _, ThreadArchitecture as _,
    TimerArchitecture as _,
};
use crate::kernel::event::{EventFlagsApi, EventLoopApi as _};
use crate::kernel::vfs::filesystem::block::BlockClientApi as _;
#[cfg(feature = "virtualization")]
use crate::arch::traits::VirtualizationArchitecture as _;

pub(crate) mod arch;
pub(crate) use vfs::filesystem::block as block;
pub(crate) mod credentials;
pub(crate) mod debug;
pub(crate) mod driver;
pub(crate) mod dl;
#[allow(dead_code)]
pub(crate) mod script;
pub(crate) use script::{beam, ebpf};
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
pub(crate) mod random;
pub(crate) mod signal;
pub(crate) mod syscall;
pub(crate) use vfs::filesystem::tty as tty;
#[cfg(not(test))]
#[path = "../runtime/run.rs"]
pub(crate) mod runtime;
#[cfg(test)]
pub(crate) mod runtime {
    pub(crate) fn run(_context: super::KernelContext<'_>) {}
}
pub(crate) mod scheduler;
#[cfg(feature = "virtualization")]
pub(crate) use vfs::filesystem::vm as vm;
#[allow(dead_code)]
pub(crate) mod vfs;

// Preserve existing internal paths while modules move to the crate root.
pub(crate) mod kernel {
    #[allow(unused_imports)]
    pub(crate) use crate::{arch, beam, block, credentials, debug, dl, driver, ebpf, error, event, ipc, lib, memory, net, namespace, process, runtime, scheduler, script, signal, syscall, tty, vfs};
    #[cfg(feature = "virtualization")]
    #[allow(unused_imports)]
    pub(crate) use crate::vm;
}

pub(crate) struct KernelContext<'a> {
    pub(crate) events: &'a mut event::EventLoop<16>,
    pub(crate) block_device: Option<&'a mut block::BlockClient>,
    pub(crate) network_stack: Option<&'a mut net::stack::NetStack>,
    pub(crate) network_registration: Option<event::EventSourceToken>,
}

struct Kernel;

impl Kernel {
    fn interrupt() {
        let pending = vfs::filesystem::virtio::transport::dispatch_interrupts();
        event::EventLoop::<16>::record_device_interrupts(pending);
        vfs::filesystem::virtio::console::Console::handle_interrupt(pending);
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
        debug::debug("kz: boot info ready\n");
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
        debug::debug("kz: memory map ready\n");
        if !memory::physical::PhysicalAllocator::initialize() {
            debug::debug("kz: physical allocator initialization failed\n");
            Kernel::halt();
        }
        debug::debug("kz: physical allocator ready\n");
        memory::heap::Heap::initialize();
        debug::debug("kz: heap ready\n");
        #[cfg(feature = "virtualization")]
        let _host_virtualization = arch::VirtualizationArchitecture::initialize_host();
        assert!(
            scheduler::thread::ThreadManager::initialize(),
            "unable to initialize the thread scheduler"
        );
        debug::debug("kz: scheduler ready\n");
        assert!(
            arch::ThreadArchitecture::initialize_thread_local_storage(0),
            "unable to initialize thread-local storage"
        );
        debug::debug("kz: kernel TLS ready\n");
        memory::dma::DmaArena::initialize();
        assert!(
            arch::InterruptArchitecture::initialize_interrupt_controller(),
            "unable to initialize the interrupt controller"
        );
        debug::debug("kz: interrupt controller ready\n");
        arch::TimerArchitecture::initialize_timer();

        let console_ready = vfs::filesystem::virtio::console::Console::initialize();
        debug::debug("kz: virtio console initialized\n");
        let mut virtio_services = vfs::filesystem::virtio::service::KernelVirtioServices::initialize();
        debug::debug("kz: virtio block/network initialized\n");
        debug::debug("kz: initializing host VirtIO-FS\n");
        let host_virtio_fs = vfs::filesystem::virtio::fs::VirtioFs::initialize_blocking();
        debug::debug("kz: host VirtIO-FS initialization returned\n");
        let virtio_fs_root = host_virtio_fs
            .as_ref()
            .map(|filesystem| filesystem.root());
        match vfs::mount_virtio_devices(virtio_fs_root) {
            Ok(_) if host_virtio_fs.is_some() => {
                debug::debug("kz: host VirtIO-FS mounted at /fs\n");
            }
            Ok(_) => debug::debug("kz: VFS initialized without host VirtIO-FS\n"),
            Err(_) => debug::debug("kz: VFS initialization failed\n"),
        }
        let mut events = event::EventLoop::<16>::new();
        let network_registration = events
            .register(
                net::stack::NETWORK_EVENT_SOURCE,
                event::EventFlags::READABLE,
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
        if console_ready && vfs::filesystem::virtio::console::Console::write(b"kz: virtio-console transmit OK\n") {
            debug::write_virtio_devices();
            if host_virtio_fs.is_some() {
                match scheduler::thread::boot::launch_busybox() {
                    Ok(()) => debug::debug("kz: BusyBox launch returned unexpectedly\n"),
                    Err(error) => debug::debug_fmt(format_args!(
                        "kz: BusyBox launch failed: {:?}\n",
                        error
                    )),
                }
            }
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

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    Kernel::halt()
}
