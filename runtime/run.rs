#[path = "ffi.rs"]
mod runtime_ffi;
#[path = "libc/mod.rs"]
mod runtime_libc;
#[path = "v8.rs"]
mod runtime_v8;

pub(crate) fn run(events: &mut crate::event_loop::EventLoop<16>) {
    if crate::runtime::runtime_v8::page_allocator_smoke() {
        let _page_allocator = crate::console::write(b"kz: V8 page allocator hooks OK\n");
    }
    match crate::runtime::runtime_v8::run_smoke(events) {
        Some(0) => {}
        Some(_) => {
            let _v8_error = crate::console::write(b"kz: V8 smoke failed\n");
        }
        None => {
            let _v8_status = crate::console::write(b"kz: V8 not integrated\n");
        }
    }
}
