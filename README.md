<!--
Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved

Permission for non-commercial use is hereby granted,
free of charge, without warranty of any kind.
-->

# kz

*A tiny kernel with big curiosity.*

`kz` is an experimental, freestanding Rust kernel for x86_64 and AArch64. It
boots directly in QEMU and is a playground for kernel foundations: memory,
interrupts, scheduling, VirtIO devices, a small VFS, and JavaScript with V8.

## What's inside

- A `no_std` Rust kernel with architecture-specific boot, paging, interrupts,
  timers, and kernel, userspace, and optional virtual-machine thread contexts.
- User-address-space and ELF-loading primitives, plus an early syscall ABI for
  process/thread exit, IDs, yielding, console writes, clocks, and memory mapping.
- Physical and virtual memory management, a kernel heap, and a bounded DMA
  arena.
- VirtIO device support for console, block, network, input, GPU, SCSI, RTC, and
  filesystem devices.
- A bounded task executor, a timer-preemptive pthread subset, and an event loop
  for timers, readiness, completions, and device interrupts.
- Feature-gated virtualization support with vCPU contexts integrated into the
  thread scheduler.
- A small internal VFS with a device metadata tree and VirtIO-FS mount support.
- An embedded V8 runtime, a C/C++ bridge, and a small freestanding C library.
- A TypeScript VFAT reader/writer for FAT16 and FAT32 volumes, used to load
  JavaScript modules from a block device.

## Build and boot

The repository selects stable Rust and the `x86_64-unknown-none` and
`aarch64-unknown-none` targets in `rust-toolchain.toml`. You will also need
Clang, `llvm-objcopy`, Make, Ninja, Node.js/npm, Python 3, and QEMU's x86_64 and
AArch64 system emulators.

Build both kernels and run the smoke tests:

```sh
make test
```

This builds the TypeScript app and both kernel images, checks the VFAT module,
then boots the x86_64 and AArch64 QEMU smoke tests. The x86_64 test also attaches
a temporary disk and network device to check VFAT app loading, block I/O, and a
TCP echo. Both architectures also smoke-test kernel-thread scheduling and
joining, plus user VM stack and mapping allocation, access protection, and
release. The AArch64 smoke covers boot and the embedded runtime with a console.

For an interactive QEMU run, use one target at a time:

```sh
make run-x86_64
make run-aarch64
```

To build an image without starting QEMU, use
`make build/x86_64/kernel.elf` or `make build/aarch64/kernel.bin`.

## A small map

- `kernel/` contains architecture code, memory management, scheduling, events,
  drivers, networking, and the VFS.
- `runtime/` contains the V8 embedder, host bridge, and freestanding C library.
- `app/` contains the TypeScript application and VFAT support.
- `tests/` contains the VFAT and QEMU smoke tests.

## Scope

This is a kernel lab, not a general-purpose OS. It has user-mode, ELF, syscall,
and optional guest-virtualization foundations, but not a general process manager
or program-launch workflow. JavaScript runs inside the kernel's V8 embedder, and
the QEMU smoke tests are the current way to exercise the system end to end.
