# kz

A bootable first milestone toward a Rust monolithic kernel with embedded V8.
This is not yet a JavaScript runtime or a complete operating system.

## Implemented

- Freestanding `no_std` Rust kernel on x86_64 and AArch64.
- x86_64 Multiboot entry, long-mode transition, and first-4-GiB identity mapping.
- AArch64 direct QEMU boot with the MMU disabled and an EL2-to-EL1 transition
  when required.
- One shared modern VirtIO-MMIO console driver with a split transmit queue,
  identity-addressed DMA buffers, memory barriers, and completion polling.
- QEMU smoke tests for both architectures. Success requires repeated queue
  submissions and guest-observed completions, not just a successful build.

There is no kernel-side ELF loader, filesystem, process scheduler, or libc.
Boot assembly and linker scripts are architecture-specific; the console code
is shared. Only the VirtIO console is used for kernel output.

## Build and test

Required: Rust with `x86_64-unknown-none` and `aarch64-unknown-none` installed,
Clang, LLD, llvm-objcopy, Make, Python 3, and QEMU's x86 and ARM system emulators.

```sh
rustup target add x86_64-unknown-none aarch64-unknown-none
make test
```

The Makefile defaults to `$HOME/.cargo/bin/rustc`. Override with
`make RUSTC=/path/to/rustc test` if needed. This prototype uses direct rustc
object compilation rather than Cargo to combine Rust with boot assembly.

The test runs each guest for up to five seconds and then stops it. A healthy
guest halts after sending:

```text
kz: x86_64 Rust kernel booted
kz: virtio-console transmit OK
kz: V8 not integrated
```

The ARM guest prints `aarch64` instead of `x86_64`.

## Interactive runs

```sh
make run-x86_64
make run-aarch64
```

Run one command at a time. Press Ctrl-A then X to leave QEMU's stdio console.
Both machines use software emulation, so KVM privileges are unnecessary.

The x86 build is linked as ELF64 and converted to an ELF32 container for
QEMU's Multiboot loader. It still executes 64-bit Rust after the boot
transition. AArch64 uses a flat kernel binary. Neither path loads native
applications inside the running kernel.

## Limitations and next milestones

This supports only the exact single-CPU QEMU configurations in the Makefile.
VirtIO-MMIO addresses are fixed for QEMU microvm and virt; there is no general
ACPI/device-tree discovery. Interrupts remain masked, DMA addresses equal
physical addresses, and the queue is polled. There is no exception recovery,
dynamic allocator, timer service, memory isolation, or production safety.

V8 is **not** downloaded, linked, initialized, or simulated. No JavaScript
has been executed. The bare-metal V8 port remains the major unresolved work:

1. Pin V8 and prove a freestanding build with its required C/C++ support.
2. Add heap and page allocation, clocks, and a worker-free V8 platform/event
   loop appropriate to that version. Start JITless.
3. Implement a narrow C ABI bridge and execute embedded JavaScript with a
   console binding and explicit microtask processing.
4. Add VirtIO-block requests and raw disk promise bindings, without a filesystem.
5. Add VirtIO-net, a no_std network stack, and asynchronous TCP bindings.

V8 stays statically linked into the image. Kernel threads and ELF application
loading are not part of the initial design. Running V8 at kernel privilege
means native runtime bugs can compromise the whole system.