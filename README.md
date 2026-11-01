# kz

A bootable first milestone toward a Rust monolithic kernel with embedded V8.
This is not yet a JavaScript runtime or a complete operating system.

## Implemented

- Freestanding `no_std` Rust kernel on x86_64 and AArch64.
- x86_64 Multiboot entry, long-mode transition, and identity mapping up to
  64 GiB.
- AArch64 direct QEMU boot with 39-bit identity translation and an EL2-to-EL1
  transition when required.
- Bootstrap MMU mappings for the QEMU device window and RAM, using
  non-cacheable RAM for VirtIO DMA compatibility. RAM extents come from the
  Multiboot map or AArch64 device tree.
- Architecture interrupt vectors, x86 APIC/IOAPIC routing, AArch64 GICv2
  routing, and counter-based timers.
- A bitmap frame allocator, coalescing 256 KiB kernel heap, and bounded DMA
  arena for the configured QEMU RAM maps.
- Shared modern VirtIO-MMIO transport and split queues, with nonblocking
  interrupt-driven console, block, and Ethernet paths plus a polled RNG.
- A smoltcp IPv4/TCP adapter and a readiness-registered event loop with a
  fixed-capacity timer heap, completion queue, device interrupt events, and C
  ABI console/time functions.
- Freestanding C `stdio` and memory/string subsets shared by x86_64 and
  AArch64; `printf`, `puts`, and `putchar` write to the VirtIO console.
- Freestanding C `malloc` and `free` backed by the kernel heap.
- An anonymous Linux-shaped `mmap`/`munmap` subset backed by physical frames.
- A bounded, cooperative pthread subset with fixed stacks, join/detach, mutexes,
  and condition variables.
- QEMU smoke tests for both architectures. They verify interrupt-driven console
  completions, memory discovery, `mmap`, pthreads, C allocation, and stdio output.

There is no kernel-side ELF loader, filesystem, or full libc. Kernel threads
are cooperatively scheduled on the single configured CPU. Boot assembly,
paging and interrupt vectors are architecture-specific; the VirtIO transport
and drivers are shared. Only the VirtIO console is used for kernel output.

## Build and test

Required: stable Rust with Cargo and both `x86_64-unknown-none` and
`aarch64-unknown-none` targets, Clang, LLD, llvm-objcopy, Make, Python 3, and
QEMU's x86 and ARM system emulators.

```sh
rustup target add x86_64-unknown-none aarch64-unknown-none
make test
```

The root Cargo workspace contains the freestanding kernel package.
`rust-toolchain.toml` selects stable Rust and the two targets. The kernel's
build script assembles architecture entry code with Clang and applies the
architecture-specific linker script. Make packages the resulting x86 ELF and
AArch64 binary, then runs the QEMU smoke tests.

The test runs each guest for up to five seconds and then stops it. A healthy
x86_64 guest emits:

```text
kz: x86_64 Rust kernel booted
kz: virtio-console transmit OK
kz: C stdio printf 42 hello 0x2a
kz: C stdio puts works
!
kz: RAM above 128 MiB detected
kz: mmap smoke OK
kz: malloc/free smoke OK
kz: RAM above 4 GiB mapped
kz: pthreads smoke OK
kz: V8 page allocator hooks OK
kz: V8 not integrated
```

The ARM guest prints `aarch64` instead of `x86_64`, omits the 4 GiB and V8 page
allocator lines, and reports its own memory, mmap, malloc, and pthread smoke results.

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

This supports only the single-CPU QEMU configurations in the Makefile. RAM
regions are discovered from Multiboot or FDT and managed up to the 64 GiB
physical-address ceiling. VirtIO-MMIO windows remain fixed; there is no general
ACPI/device-tree device discovery. The frame allocator reserves memory through
`__bss_end`. The 256 KiB heap reclaims and coalesces freed blocks; the DMA arena
remains a bounded bump allocator. DMA addresses equal physical addresses.
The drivers poll and allow one outstanding queue request; the default smoke
tests do not attach block, network, or RNG devices. The network stack uses a
static `10.0.2.15/24` address and the QEMU user-network gateway. There is no
interrupt-controller setup, address-space manager, filesystem, exception
recovery, memory isolation, or production safety. The pthread subset has eight
fixed thread slots, 16 KiB stacks, and no preemption, SMP, cancellation, or
thread-local storage; it is not a complete POSIX threads implementation.
Thread attributes accept only the default or fixed stack size. Contended
mutex, condition, and join operations yield cooperatively and may return
`EDEADLK` when no runnable thread can make progress.
The `mmap` subset supports anonymous private/shared mappings in the shared
kernel identity map only. It has no file mappings, `MAP_FIXED`, partial
`munmap`, or per-process address spaces, and is not a Linux `mmap(2)` syscall.

V8 is not linked into, initialized in, or simulated by the kernel. No
JavaScript has been executed in the kernel. V8 15.9.62 is pinned in
`runtime/v8/version.txt` and checked out at `runtime/v8/src`. The x64 Linux
host build produces a monolithic static archive at
`runtime/v8/src/out.gn/x64.release/obj/libv8_monolith.a`; its JIT is enabled
with Sparkplug, Maglev and TurboFan, and JITless mode is disabled. Rebuild it
from the repository root with:

```sh
cd runtime/v8/src
buildtools/linux64/gn gen out.gn/x64.release --args="$(<../args.gn)"
ninja -C out.gn/x64.release -j4 v8_monolith
```

Run the host embedder and JIT checks with:

```sh
make v8-host-test
```

This builds and runs V8's `v8_hello_world` embedder sample, then verifies an
optimized JavaScript function in `d8`. The JIT smoke uses V8's internal test
intrinsics and status bits, so it is intentionally tied to the pinned version.
These are hosted x64 checks; V8 is not linked into the kernel, and this is not
a freestanding build. The bare-metal V8 port remains the major unresolved work:

1. Port the required C/C++ runtime and V8 platform services; provide safe
  executable-memory support before enabling JIT in the kernel.
2. Execute embedded JavaScript with console, timer, TCP and raw block bindings,
  including explicit microtask processing.
3. Add device-backed QEMU tests for block, network and RNG I/O.

The intended design statically links V8 into the image. Preemptive or SMP
threads and ELF application loading are not part of the initial design. Running
V8 at kernel privilege means native runtime bugs can compromise the whole system.