<!--
Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved

Permission for non-commercial use is hereby granted,
free of charge, without warranty of any kind.
-->

# Kernel Components Missing Behind POSIX ENOSYS

This list records kernel or runtime-kernel facilities indicated by `ENOSYS` stubs in `runtime/bridge/posix`. It excludes libc algorithms and APIs that can be implemented over existing kernel facilities.

## Process Lifecycle

- [ ] Add POSIX process identity and lifecycle: stable process IDs, parent/child relationships, fork-style creation, exec transitions, waitable exit status and reaping. Wire it to `fork`/`exec*`, `posix_spawn*`, `wait*`, and process-ID APIs in [`runtime/bridge/posix/unistd.rs`](runtime/bridge/posix/unistd.rs), [`runtime/bridge/posix/spawn.rs`](runtime/bridge/posix/spawn.rs), and [`runtime/bridge/posix/sys/wait.rs`](runtime/bridge/posix/sys/wait.rs). Reuse the existing process objects and ELF loading machinery rather than adding a second loader.
- [ ] Add process groups, sessions, and controlling-terminal ownership for `setpgid`, `setsid`, and related APIs.
- [ ] Expose credential changes and queries (`setuid`/`setgid` families) through validated process operations; the VFS already has user/group credential and node-metadata types.

## Signals and Waits

- [ ] Add signal dispositions, per-thread masks, pending queues, process-directed delivery, and signal-aware blocking waits. This unblocks the `signal`/`sigaction` family, `kill`, `raise`, `sigwait*`, `pause`, and `alarm` in [`runtime/bridge/posix/signal.rs`](runtime/bridge/posix/signal.rs) and [`runtime/bridge/posix/unistd.rs`](runtime/bridge/posix/unistd.rs).
- [ ] Connect signal delivery to process exit, child-state changes, timers, and interruptible I/O so wait APIs can report interruption and restart behavior.

## VFS and Descriptors

- [ ] Add per-process filesystem context (current directory and umask) plus directory-descriptor-relative path resolution. Complete `openat` and the dirfd cases of `fstatat`, `mkdirat`, and `unlinkat`.
- [ ] Add descriptor duplication, pipes, and the regular-file descriptor operations required by `fcntl`, `pread`/`pwrite`, `truncate`/`ftruncate`, and file locking. Reuse plugin positioned I/O where possible.
- [ ] Complete VFS mutation and backing-filesystem hooks for hard links, symbolic links, timestamps, FIFOs, and device nodes. Wire existing metadata setters for chmod/chown operations; add truncate and persistent sync hooks where backends support them.
- [ ] Expose filesystem allocation and capacity information for `posix_fallocate` and `statvfs`/`fstatvfs`.
- [ ] Provide stable device/inode identifiers for `ftok` and keyed IPC lookup.

## Memory and IPC

- [ ] Add page pinning and unpinning for `mlock`/`munlock`, including process-wide variants, and define backing-object synchronization semantics for `msync`.
- [ ] Add POSIX shared-memory object creation, naming, unlink, and mapping lifecycle for `shm_open`/`shm_unlink`.
- [ ] Add a namespaced IPC object registry with keys and validated identifiers, then implement System V message queues, semaphore sets, and shared-memory attach/detach/control operations. Existing endpoint and shared-memory capabilities do not provide these POSIX object semantics.
- [ ] Add named POSIX semaphore storage and timed waits, and POSIX message queues with blocking/timed send and receive, notification, and attribute operations.

## Scheduling and Accounting

- [ ] Expose scheduling policy and priority changes/queries, including real-time policies and round-robin intervals, for the `sched_*` and pthread scheduling APIs.
- [ ] Add realtime clock adjustment policy for `clock_settime`, keeping realtime updates separate from the monotonic clock used for elapsed-time waits.
- [ ] Add resource-limit state and accounting for CPU time, child usage, priorities, and process/thread clocks. Wire `getrlimit`/`setrlimit`, `getrusage`, `times`, `getpriority`, and CPU-clock queries.
- [ ] Add deadline-based wait and cancellation support for timed pthread condition/mutex waits and cancellation points; keep barriers, rwlocks, spinlocks, and attribute bookkeeping in the userspace synchronization layer where possible.
- [ ] Add POSIX timer object lifecycle and notification semantics on top of the existing event timer queue; implement create/delete/get/set/overrun operations.
- [ ] Add asynchronous I/O request tracking, completion results, cancellation, and suspend/wait operations for the POSIX AIO APIs.

## Devices and Networking

- [ ] Add a TTY line discipline, terminal attributes and window size, PTYs, and controlling-terminal operations. The current console path provides byte I/O only.
- [ ] Add generic descriptor-based device-control dispatch for `ioctl`/`posix_devctl` and the terminal operations that depend on it.
- [ ] Add local-domain sockets and `socketpair`, plus ancillary-message and out-of-band socket support needed by the remaining socket stubs.
- [ ] Add a kernel cryptographically secure random-byte service and expose it to `getentropy`.

## Existing Facilities to Wire

- [ ] Adapt `select`/`pselect` to descriptor readiness and the existing poll/event interfaces; this is primarily POSIX bridge integration, not a new event engine.
- [ ] Implement POSIX `DIR` stream wrappers over VFS directory iteration; the missing work is the descriptor-backed stream contract and libc bridge.
- [ ] Integrate `dlopen`/`dlsym`/`dlclose` with the existing ELF loading and symbol machinery instead of treating dynamic loading as a wholly absent kernel subsystem.
- [ ] Connect POSIX timers to the existing timer queue and runtime scheduling/cancellation hooks rather than adding another timer facility.

Math, floating-point environment, regex, locale/formatting, account and group databases, DBM, search helpers, and shell-style expansion also have unsupported POSIX entry points, but those are libc or userspace-service work rather than missing kernel components.