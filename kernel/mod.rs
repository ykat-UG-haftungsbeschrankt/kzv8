use core::sync::atomic::{AtomicBool, Ordering};

pub(crate) mod arch;
pub(crate) mod block;
pub(crate) mod drivers;
pub(crate) mod event;
pub(crate) mod memory;
pub(crate) mod net;
pub(crate) mod scheduler;
#[path = "../runtime/run.rs"]
pub(crate) mod runtime;

static FUTURE_SMOKE_DONE: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "C" fn kz_handle_virtio_interrupt() {
    let pending = drivers::virtio::transport::dispatch_interrupts();
    event::r#loop::record_device_interrupts(pending);
    drivers::virtio::console::Console::handle_interrupt(pending);
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
    memory::virtual_memory::initialize();
    assert!(
        memory::physical::PhysicalAllocator::initialize(),
        "unable to initialize physical memory"
    );
    memory::heap::Heap::initialize();
    scheduler::thread::initialize();
    assert!(
        arch::initialize_thread_local_storage(0),
        "unable to initialize thread-local storage"
    );
    memory::dma::DmaArena::initialize();
    assert!(
        arch::initialize_interrupt_controller(),
        "unable to initialize the interrupt controller"
    );
    arch::timer::initialize();
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
    let console_ready = drivers::virtio::console::Console::initialize();
    let future_smoke_started = scheduler::task::spawn(futures_smoke()).is_ok();
    if future_smoke_started {
        scheduler::task::run_until_stalled();
    }
    arch::interrupts::enable();
    if console_ready {
        if drivers::virtio::console::Console::write(arch::BOOT_MESSAGE)
            && drivers::virtio::console::Console::write(b"kz: virtio-console transmit OK\n")
        {
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