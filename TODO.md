<!--
Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved

Permission for non-commercial use is hereby granted,
free of charge, without warranty of any kind.
-->

# Kernel Components Missing Behind POSIX ENOSYS

This list records kernel or runtime-kernel facilities indicated by `ENOSYS` stubs in `runtime/bridge/posix`. It excludes libc algorithms and APIs that can be implemented over existing kernel facilities.

## Process Lifecycle

- [ ] Add the remaining POSIX process lifecycle: fork-style creation, exec transitions, and `posix_spawn*`. Parent-owned child events and waitable exit/stop/continue status now back `wait`, `waitpid`, and `waitid` in [`kernel/process.rs`](kernel/process.rs) and [`runtime/bridge/posix/sys/wait.rs`](runtime/bridge/posix/sys/wait.rs). Fork and exec still need address-space cloning/replacement and userspace entry setup; reuse the existing process objects and ELF loading machinery rather than adding a second loader.
- [x] Add validated POSIX process IDs, parent IDs, process groups, and sessions for `getpgid`, `getpgrp`, `getppid`, `getsid`, `setpgid`, `setpgrp`, and `setsid`. The boot console is a controlling TTY with canonical/raw line discipline, termios attributes, window size, foreground process-group operations, and `/dev/tty`/`/dev/console` descriptors; PTYs remain unsupported. See [`Process`](kernel/process.rs), [`TTY state`](kernel/tty.rs), and [`POSIX bridge`](runtime/bridge/posix/unistd.rs).
- [x] Expose validated real/effective/saved UID and GID changes and queries, bounded supplementary groups, and use current process credentials for POSIX filesystem open/create checks and ownership metadata. Privileged identity changes require effective UID 0; unprivileged changes are limited to real/effective/saved identities. See [`process credentials`](kernel/process.rs), [`POSIX identity APIs`](runtime/bridge/posix/unistd.rs), and [`runtime filesystem`](runtime/libc/filesystem.rs).

## Signals and Waits

- [x] Add signal dispositions, per-thread masks, pending queues, process-directed delivery, and signal-aware blocking waits. This wires the `signal`/`sigaction` family, `kill`, `raise`, `pthread_kill`, `sigwait*`, `pause`, and `alarm` through [`runtime/bridge/posix/signal.rs`](runtime/bridge/posix/signal.rs), [`runtime/bridge/posix/unistd.rs`](runtime/bridge/posix/unistd.rs), and [`runtime/libc/posix.rs`](runtime/libc/posix.rs). `sigaltstack`/`SA_ONSTACK` and architecture-managed asynchronous signal frames remain unsupported; handlers run at runtime signal checkpoints.
- [x] Connect delivery to process termination, child exit/stop/continue changes, alarm timers, and interruptible console, socket, packet, `poll`/`ppoll`, and `nanosleep` waits. Supported waits report `EINTR` or retry according to `SA_RESTART`; process wait/reaping remains in the separate lifecycle TODO.

## VFS and Descriptors

- [x] Add per-process filesystem context (current directory and umask) plus directory-descriptor-relative path resolution. `openat`, `fstatat`, `mkdirat`, and `unlinkat` resolve relative paths from a validated directory descriptor; `chdir`, `fchdir`, and `getcwd` use the process context.
- [ ] Add file locking and complete descriptor duplication across sockets and standard descriptors. Runtime pipes now support blocking/nonblocking `pipe`/`pipe2`, duplicated endpoints, `POLLIN`/`POLLOUT`/hangup readiness, and EOF/`EPIPE`; regular-file `dup*`, `F_DUPFD*`, descriptor flags, `fcntl` status flags, `pread`/`pwrite`, `truncate`/`ftruncate`, and `posix_fallocate` use the runtime file table.
- [ ] Complete VFS mutation and backing-filesystem hooks for hard links, symbolic links, timestamps, FIFOs, and device nodes. `chmod`/`chown` path and descriptor forms now use the existing metadata setters; persistent sync hooks remain backend-dependent.
- [ ] Expose filesystem capacity information for `statvfs`/`fstatvfs`; `posix_fallocate` now reserves and extends in-memory regular-file data through the runtime filesystem.
- [x] Provide stable device/inode identifiers for `ftok` and keyed IPC lookup. The runtime filesystem has stable device/inode metadata, and `ftok` now composes keys from it.

## Memory and IPC

- [ ] Add page pinning and unpinning for `mlock`/`munlock`, including process-wide variants, and define backing-object synchronization semantics for `msync`.
- [ ] Add POSIX shared-memory object creation, naming, unlink, and mapping lifecycle for `shm_open`/`shm_unlink`.
- [ ] Add a namespaced IPC object registry with keys and validated identifiers, then implement System V message queues, semaphore sets, and shared-memory attach/detach/control operations. Existing endpoint and shared-memory capabilities do not provide these POSIX object semantics.
- [ ] Add POSIX message queues with blocking/timed send and receive, notification, and attribute operations. Named POSIX semaphore storage, close/unlink lifetime, and timed waits now use the runtime registry and clock hooks.

## Scheduling and Accounting

- [ ] Expose scheduling policy and priority changes/queries, including real-time policies and round-robin intervals, for the `sched_*` and pthread scheduling APIs.
- [x] Add realtime clock adjustment policy for `clock_settime`, keeping realtime updates separate from the monotonic clock used for elapsed-time waits.
- [ ] Add resource-limit state and accounting for CPU time, child usage, priorities, and process/thread clocks. Wire `getrlimit`/`setrlimit`, `getrusage`, `times`, `getpriority`, and CPU-clock queries.
- [ ] Add cancellation-point support; timed pthread condition and mutex waits now convert absolute clock deadlines to the existing monotonic scheduler clock and reacquire the mutex before returning. Keep barriers, rwlocks, spinlocks, and attribute bookkeeping in the userspace synchronization layer where possible.
- [x] Add POSIX timer object lifecycle and notification semantics on top of the existing event timer queue; implement create/delete/get/set/overrun operations.
- [ ] Add asynchronous I/O request tracking, completion results, cancellation, and suspend/wait operations for the POSIX AIO APIs.

## Devices and Networking

- [ ] Add pseudoterminals (PTYs) with independent master/slave device state and session integration.
- [ ] Add generic descriptor-based device-control dispatch for `ioctl`/`posix_devctl` and the terminal operations that depend on it.
- [ ] Add local-domain sockets and `socketpair`, plus ancillary-message and out-of-band socket support needed by the remaining socket stubs.
- [x] Add a kernel cryptographically secure random-byte service and expose it to `getentropy` using the architecture-gated RDRAND/RNDR source.

## Existing Facilities to Wire

- [x] Adapt `select`/`pselect` to descriptor readiness and the existing poll/event interfaces; this is primarily POSIX bridge integration, not a new event engine.
- [x] Implement POSIX `DIR` stream wrappers over VFS directory iteration; the missing work is the descriptor-backed stream contract and libc bridge.
- [x] Integrate `dlopen`/`dlsym`/`dlclose` with the existing ELF loading and symbol machinery in [`runtime/bridge/posix/dlfcn.rs`](runtime/bridge/posix/dlfcn.rs), including dependency loading, relocation scopes, process-scoped opaque handles, and release of unreachable mappings. Relocation is eager for both `RTLD_LAZY` and `RTLD_NOW`; the symbol scope contains loaded dependencies and `RTLD_GLOBAL` objects, not a separately registered main-executable symbol table.
- [x] Connect POSIX timers to the existing event timer queue and runtime scheduling/cancellation hooks in [`runtime/bridge/posix/time.rs`](runtime/bridge/posix/time.rs) and [`runtime/ffi.rs`](runtime/ffi.rs), including `SIGEV_NONE`, `SIGEV_SIGNAL`, `SIGEV_THREAD`, absolute/relative and periodic deadlines, querying, deletion, and overrun counts.

Math, floating-point environment, regex, locale/formatting, account and group databases, DBM, search helpers, and shell-style expansion also have unsupported POSIX entry points, but those are libc or userspace-service work rather than missing kernel components.