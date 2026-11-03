# kz

**An experimental freestanding Rust kernel with an embedded V8 runtime on x86_64.**

An AArch64 boot path is also present, but V8 and the device-backed JavaScript
smoke run only on x86_64. This is a kernel prototype, not a general-purpose OS.

| Target | Status |
| --- | --- |
| x86_64 | Builds with V8 and passes the device-backed QEMU smoke test. |
| AArch64 | Boot path and smoke harness are present; V8 is not integrated. |

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
- Modern VirtIO-MMIO console, block, and Ethernet drivers with DMA-backed
  queues and interrupt-driven device completion.
- A smoltcp IPv4/TCP adapter and a readiness-registered event loop with a
  fixed-capacity timer heap, completion queue, device interrupt events, and C
  ABI console/time functions.
- Freestanding C `stdio` and memory/string subsets shared by x86_64 and
  AArch64; `printf`, `puts`, and `putchar` write to the VirtIO console.
- Freestanding C `malloc` and `free` backed by the kernel heap.
- An anonymous Linux-shaped `mmap`/`munmap` subset backed by physical frames.
- A bounded, cooperative pthread subset with fixed stacks, join/detach, mutexes,
  and condition variables.
- A freestanding x86_64 V8 embedder with a cooperative task runner and kernel
  page allocator. Its built-in JavaScript smoke exercises console, timers,
  event-loop, raw block, and TCP bindings, with explicit microtask checkpoints.
- A device-backed x86_64 QEMU smoke test for block read/write and TCP echo, plus
  kernel, allocator, libc, and V8 checks. AArch64 does not integrate V8 or
  attach block/network devices in its smoke path.

There is no kernel-side ELF loader, on-disk filesystem, or full libc. Kernel
threads are cooperatively scheduled. Boot assembly, paging, and interrupt
vectors are architecture-specific; VirtIO transport and drivers are shared.
The VirtIO console is the kernel's output device.

## Build and test

Required: stable Rust with Cargo and the `x86_64-unknown-none` and
`aarch64-unknown-none` targets, Clang, llvm-objcopy, Make, Ninja, Python 3, and
QEMU's x86_64 and AArch64 system emulators. The x86_64 build also generates and
builds the pinned V8 monolith with the repository's GN/Ninja setup.

```sh
rustup target add x86_64-unknown-none aarch64-unknown-none
make test
```

Build an image directly with `make build/x86_64/kernel.elf` or
`make build/aarch64/kernel.bin`. The x86_64 build and device-backed QEMU smoke
pass. The current AArch64 build is blocked by `c_char` type mismatches in
`runtime/libc/compat.rs`, so `make test` stops before booting that guest.

For an interactive x86_64 boot, run `make run-x86_64`; for AArch64, use
`make run-aarch64` after its build is available. These targets attach a console
only. `make test` is the route that attaches the x86_64 disk and network test
devices.

The x86 context-switch FPU format defaults to SSE. Select another mode with
`make FPU_MODE=x87`, `make FPU_MODE=sse`, or `make FPU_MODE=avx`. AVX mode
requires XSAVE and AVX support on every CPU and halts during startup otherwise.
The x87 mode saves only x87 state; it is unsafe for code that uses SSE, including
the current x86_64 V8 runtime. The AArch64 SIMD context is unaffected by
`FPU_MODE`.

For AArch64, the full SIMD context is saved by default. Set
`AARCH64_FPU_STATE=off` to skip the full per-thread SIMD image; the switch still
saves ABI-preserved `d8`-`d15`. Interrupt entry continues to preserve the full
SIMD state.

The root Cargo manifest defines the freestanding kernel package, and
`rust-toolchain.toml` selects stable Rust and both targets. The build script
assembles architecture entry code with Clang and applies the matching linker
script. Make packages the x86 ELF and AArch64 binary before running QEMU.

The smoke runner captures each guest for up to 30 seconds. The x86_64 test
attaches a temporary 16 MiB raw disk, seeds sector 8, then verifies a read,
write, and read-back on sectors 8 and 9. It also attaches QEMU user networking
and echoes a four-byte TCP payload through a host listener on port `17777`.
Both checks run through the embedded JavaScript bindings.

A successful x86_64 run includes these runtime markers:

```text
kz: first lazy page fault resolved
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
kz: V8 smoke entered
kz: V8 flags set
kz: V8 platform created
kz: V8 platform initialized
kz: V8 initialized
kz: V8 allocator ready
kz: V8 isolate created
kz: V8 context created
kz: V8 script compiled
kz: JavaScript console binding OK
kz: raw block binding OK
kz: TCP binding OK
kz: V8 script ran
kz: V8 event loop smoke OK
kz: V8 explicit microtasks OK
kz: JS result: 42
kz: V8 disposing isolate
kz: V8 isolate disposed
kz: V8 disposing runtime
kz: V8 runtime disposed
kz: V8 platform disposed
kz: V8 evaluated 6 * 7 = 42
```

When it builds, the AArch64 guest prints `aarch64` instead of `x86_64`, omits
the lazy-fault, V8, and 4 GiB diagnostics, and reports `kz: V8 not integrated`.

## Internals and kernel API

`kernel/main.rs` initializes memory, interrupts, timers, the optional block and
network devices, and a fixed-capacity `EventLoop<16>`. It passes the live event
loop and device handles to `runtime/run.rs`. That module starts the x86_64 V8
embedder; AArch64 continues without V8.

The JavaScript bridge is an internal C ABI, not a syscall interface:

| Layer | Responsibility |
| --- | --- |
| `runtime/bridge/src/engine.cc` | Creates the V8 context and maps JS functions to native callbacks. |
| `runtime/ffi.rs` | Validates host-call arguments and forwards event, block, and TCP operations into Rust. |
| `kernel/event/loop.rs` | Queues timers, completions, readiness, and device interrupts. |
| `kernel/block/device.rs`, `kernel/net/tcp.rs` | Submit raw block transfers and poll TCP sockets through smoltcp. |

The C ABI is exported from `runtime/ffi.rs`; these host calls are internal
embedder hooks, not syscalls available to a user process:

- Console/time: `kz_host_console_write`, `kz_host_time_ns`, and
  `kz_host_wait_for_interrupt`.
- Events: `kz_host_event_loop_poll`, `kz_host_event_loop_schedule_timer`,
  `kz_host_event_loop_cancel_timer`, `kz_host_event_loop_register`,
  `kz_host_event_loop_modify`, `kz_host_event_loop_set_readiness`,
  `kz_host_event_loop_deregister`, `kz_host_event_loop_complete`, and
  `kz_host_event_loop_next_deadline`.
- Block: `kz_host_block_capacity`, `kz_host_block_read`, and
  `kz_host_block_write`.
- TCP: `kz_host_tcp_available`, `kz_host_tcp_connect`, `kz_host_tcp_send`,
  `kz_host_tcp_receive`, and `kz_host_tcp_close`.

Block and network VirtIO rings and buffers use the bounded DMA arena so
device-visible addresses are physical addresses. Each block or network queue
permits one outstanding request at a time.

## JavaScript interface

The x86_64 embedder runs one built-in script; there is no JavaScript file loader
or REPL yet. Its current host surface is:

| Surface | API | Notes |
| --- | --- | --- |
| Surface | API | Notes |
| --- | --- | --- |
| Console | `console.log(...values)` | Converts values to text and writes a line to the VirtIO console. |
| Timers | `kz.timer.setTimeout(callback, delayMs)`, `kz.timer.clearTimeout(id)` | One-shot callbacks; timer IDs are `BigInt`. |
| Events | `kz.events.poll()`, `kz.events.scheduleTimer(delayNs)`, `kz.events.cancelTimer(id)`, `kz.events.register(sourceId, flags, data)`, `kz.events.modify(token, flags, data)`, `kz.events.setReadiness(token, flags)`, `kz.events.deregister(token)`, `kz.events.complete(id, status)`, `kz.events.nextDeadline()` | Nonblocking event-loop operations. `poll()` returns `null` or one event; IDs are `BigInt`. |
| Block | `kz.block.capacitySectors()`, `kz.block.read(sector, byteLength)`, `kz.block.write(sector, bytes)` | Raw I/O. Lengths are nonzero multiples of 512 bytes, up to 64 KiB. Reads return `Uint8Array`; capacity returns `BigInt`. Sector inputs accept `BigInt` or safe-integer `Number`. |
| TCP | `kz.tcp.connect(ipv4Bytes, port)`, `kz.tcp.send(bytes)`, `kz.tcp.receive(byteLength)`, `kz.tcp.close()` | `ipv4Bytes` is a four-byte `Uint8Array`; port is 1–65535; one connection is supported. Send/receive lengths are 1–4096 bytes; receive returns `Uint8Array`. |

`kz.block.available` and `kz.tcp.available` report whether each device is
present. Block read/write and TCP connect/send/receive operations are
synchronous, bounded prototype bindings with a five-second timeout; TCP I/O is
not Promise-based. `kz.tcp.close()` releases the active socket.
`kz.events.scheduleTimer` takes nanoseconds; `kz.timer.setTimeout` takes
milliseconds. Microtasks are explicitly processed after the script and after
each timer callback.

## Interactive runs

```sh
make run-x86_64
make run-aarch64
```

Run one command at a time. Press Ctrl-A then X to leave QEMU's stdio console.
Both machines use software emulation, so KVM privileges are unnecessary.
These interactive targets attach only the VirtIO console; block and TCP
bindings are available in the x86_64 smoke test, which supplies those devices.
The x86 target uses `microvm,acpi=off`: its interrupt routing currently assumes
the first IO-APIC and fixed VirtIO GSI assignments. QEMU's ACPI-enabled microvm
can add a second IO-APIC with a different VirtIO GSI base.

The x86 build is linked as ELF64 and converted to an ELF32 container for
QEMU's Multiboot loader. It still executes 64-bit Rust after the boot
transition. AArch64 uses a flat kernel binary. Neither path loads native
applications inside the running kernel.

## Limitations

The smoke tests use one vCPU; interactive targets request two, but the kernel
does not currently start secondary CPUs, so it runs on the boot CPU only.
Kernel threads can be time-sliced by timer interrupts; the async task executor
is cooperative. There is no general SMP scheduling. RAM regions are discovered
from Multiboot or FDT and managed up to the 64 GiB physical-address ceiling.
VirtIO-MMIO windows and interrupt routing remain QEMU-specific; there is no
general ACPI/device-tree device discovery or dynamic interrupt routing.
The frame allocator reserves memory through `__bss_end`. The 256 KiB heap
reclaims and coalesces freed blocks; the DMA arena is a bounded bump allocator
whose addresses are physical addresses. VirtIO queues allow one outstanding
request at a time. The x86_64 smoke test attaches block and network devices;
the interactive targets do not. The network stack uses a static
`10.0.2.15/24` address and the QEMU user-network gateway.

There is no per-process address space, on-disk filesystem, exception recovery,
memory isolation, or production safety. The pthread subset has eight fixed
thread slots, 16 KiB stacks, and bounded TLS keys, but no cancellation or
destructor execution; it is not a complete POSIX threads implementation.
Thread attributes accept only the default or fixed stack size. Contended
mutex, condition, and join operations yield cooperatively and may return
`EDEADLK` when no runnable thread can make progress.
The `mmap` subset supports anonymous private/shared mappings in the shared
kernel identity map only. It has no file mappings, `MAP_FIXED`, partial
`munmap`, or per-process address spaces, and is not a Linux `mmap(2)` syscall.

V8 15.9.62 is pinned in `runtime/v8/version.txt` and checked out at
`runtime/v8/src`. The x86_64 kernel links a freestanding monolithic build and
executes the built-in binding smoke. It does not load arbitrary JavaScript or
verify kernel JIT execution. AArch64 does not link V8. The
separate x64 Linux host build produces a monolithic static archive at
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
These are hosted x64 checks and are separate from the freestanding kernel build.
Kernel-side V8 remains experimental: it runs at kernel privilege, uses
synchronous device bindings, and has no user-script loading or isolated
execution environment.

The intended design statically links V8 into the image. Preemptive or SMP
threads and ELF application loading are not part of the initial design. Running
V8 at kernel privilege means native runtime bugs can compromise the whole system.