<!--
Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved

Permission for non-commercial use is hereby granted,
free of charge, without warranty of any kind.
-->

# Kernel Components Missing Behind POSIX ENOSYS

This list records kernel or runtime-kernel facilities indicated by `ENOSYS` stubs in `runtime/bridge/posix`. It excludes libc algorithms and APIs that can be implemented over existing kernel facilities.

## Process Lifecycle

- [ ] Add the remaining POSIX process lifecycle: fork-style creation, exec transitions, and `posix_spawn*`. Parent-owned child events and waitable exit/stop/continue status now back `wait`, `waitpid`, and `waitid` in [`kernel/process.rs`](kernel/process.rs) and [`runtime/bridge/posix/sys/wait.rs`](runtime/bridge/posix/sys/wait.rs). `UserAddressSpace` has no private clone/replace operation and the userspace thread context records only initial entry/stack, not a resumable trap-register frame; fork/exec need those VM and architecture context contracts before a correct implementation can be wired. Reuse the existing ELF loader rather than adding a second loader.
- [x] Add validated POSIX process IDs, parent IDs, process groups, and sessions for `getpgid`, `getpgrp`, `getppid`, `getsid`, `setpgid`, `setpgrp`, and `setsid`. The boot console and PTYs support controlling-session ownership, termios attributes, window size, and foreground process-group operations; `/dev/tty` and `/dev/console` descriptors are available. See [`Process`](kernel/process.rs), [`TTY state`](kernel/tty.rs), and [`POSIX bridge`](runtime/bridge/posix/unistd.rs).
- [x] Expose validated real/effective/saved UID and GID changes and queries, bounded supplementary groups, and use current process credentials for POSIX filesystem open/create checks and ownership metadata. Privileged identity changes require effective UID 0; unprivileged changes are limited to real/effective/saved identities. See [`process credentials`](kernel/process.rs), [`POSIX identity APIs`](runtime/bridge/posix/unistd.rs), and [`runtime filesystem`](runtime/libc/filesystem.rs).

## Signals and Waits

- [x] Add signal dispositions, per-thread masks, pending queues, process-directed delivery, and signal-aware blocking waits. This wires the `signal`/`sigaction` family, `kill`, `raise`, `pthread_kill`, `sigwait*`, `pause`, and `alarm` through [`runtime/bridge/posix/signal.rs`](runtime/bridge/posix/signal.rs), [`runtime/bridge/posix/unistd.rs`](runtime/bridge/posix/unistd.rs), and [`runtime/libc/posix.rs`](runtime/libc/posix.rs). `sigaltstack`/`SA_ONSTACK` and architecture-managed asynchronous signal frames remain unsupported; handlers run at runtime signal checkpoints.
- [x] Connect delivery to process termination, child exit/stop/continue changes, alarm timers, and interruptible console, socket, packet, `poll`/`ppoll`, and `nanosleep` waits. Supported waits report `EINTR` or retry according to `SA_RESTART`; process wait/reaping remains in the separate lifecycle TODO.

## VFS and Descriptors

- [x] Add per-process filesystem context (current directory and umask) plus directory-descriptor-relative path resolution. `openat`, `fstatat`, `mkdirat`, and `unlinkat` resolve relative paths from a validated directory descriptor; `chdir`, `fchdir`, and `getcwd` use the process context.
- [x] Complete cross-table `dup2`/`dup3` targets between socket, packet, PTY, standard, and regular descriptors. A central alias registry routes arbitrary target numbers to backend-owned duplicates; shared open-description offsets and mutable status flags remain shared, while close-on-exec stays per descriptor. Backend allocators skip alias target numbers. Runtime pipes/FIFOs support blocking/nonblocking I/O, poll readiness, EOF, and `EPIPE`. Regular-file locks, `pread`/`pwrite`, `truncate`/`ftruncate`, `fcntl` flags, and `posix_fallocate` are implemented.
- [x] Add device-node creation and sync semantics for the volatile runtime VFS. `mknod`/`mknodat` create regular, FIFO, character, and block special nodes; the standard null/zero character-device numbers are serviced, while unsupported devices fail with `ENODEV`. `fsync`/`fdatasync` validate syncable filesystem descriptors and succeed because this VFS has no persistent backend; `sync` is a no-op. Hard links, bounded symbolic-link traversal, timestamps, FIFOs, and chmod/chown path and descriptor forms also use the runtime filesystem.
- [x] Expose filesystem allocation and capacity information for `posix_fallocate` and `statvfs`/`fstatvfs`. The runtime filesystem enforces 64 MiB logical data and 1024 inode limits and reports current free blocks/inodes.
- [x] Provide stable device/inode identifiers for `ftok` and keyed IPC lookup. The runtime filesystem has stable device/inode metadata, and `ftok` now composes keys from it.

## Memory and IPC


## Scheduling and Accounting

- [x] Implement optional `SCHED_SPORADIC` support. Process and pthread-thread policy/priority queries now include the sporadic-server fields in `sched_param`; validation covers high/low priorities, positive budget and replenishment period, budget no greater than period, and a bounded replenishment count. The scheduler charges execution against each server's high-priority budget, demotes exhausted servers to their configured low priority, and restores budget only at replenishment deadlines rounded up to the 10 ms scheduler tick. Replenishment storage is fixed-capacity and coalesces entries at the configured maximum. Pthread attributes preserve the full server configuration for both inherited and explicit scheduling, installed before new threads are enqueued. The scheduler chooses the highest-priority ready thread; FIFO runs until yield or higher-priority preemption, while RR rotates on ticks.
- [x] Add realtime clock adjustment policy for `clock_settime`, keeping realtime updates separate from the monotonic clock used for elapsed-time waits.
- [x] Add complete resource accounting and limit enforcement. `ProcessControlState` stores named `ResourceLimits`, and process/thread CPU, memory, I/O, signal, message, and scheduler data are grouped in resource-usage structs. Scheduler accounting separates syscall/system from user CPU; `getrusage` reports named self/child CPU, RSS, fault, logical VFS block, IPC/message, signal, and context-switch fields, and `times` includes user/system totals for waited children. Lazy page faults and unmaps update RSS/fault counts; the volatile filesystem reports successful file transfers in 512-byte logical blocks, and swap/obsolete integral-memory fields remain zero because there is no swap or persistent backing store. `RLIMIT_DATA` and `RLIMIT_AS` are enforced across syscall and legacy libc anonymous `mmap` paths, with accounting rolled back on failed mappings and adjusted for partial `munmap`. `getrlimit`/`setrlimit` store inherited soft/hard values; `RLIMIT_FSIZE` returns partial writes up to the soft limit and queues `SIGXFSZ`, `RLIMIT_CPU` queues `SIGXCPU` and kills at the hard threshold, `RLIMIT_STACK` constrains pthread creation, and backend descriptor allocation/duplication enforces `RLIMIT_NOFILE`. `getpriority`/`setpriority` support live `PRIO_PROCESS`, `PRIO_PGRP`, and `PRIO_USER` targets with permission checks.
- [x] Add asynchronous pthread cancellation. Deferred/asynchronous cancellation state and type, `pthread_testcancel`, LIFO cleanup handlers, TLS destructors on thread exit/cancellation, and cancellation points in descriptor I/O, poll/select, socket/packet, semaphore/message-queue/AIO waits, sleep, console I/O, record locks, mutex/condition waits, `pthread_join`, and child waits are implemented. Enabled asynchronous requests are prioritized and delivered at timer/preemption or thread-resumption boundaries, outside scheduler locks and IRQ-masked kernel critical sections; cancellation callbacks may run while holding application locks as permitted by POSIX async-cancel semantics. Timed pthread mutex/condition waits use monotonic deadlines.
- [x] Add POSIX timer object lifecycle and notification semantics on top of the existing event timer queue; implement create/delete/get/set/overrun operations.
- [x] Add asynchronous I/O request tracking, completion results, queued cancellation, and suspend/list-wait operations for the POSIX AIO APIs.

## Devices and Networking

- [x] Add pseudoterminals with independent bounded master/slave queues, `posix_openpt`/`/dev/ptmx`, `grantpt`/`unlockpt`, `ptsname*`, and `/dev/pts/N` opens. PTY descriptors support duplication, nonblocking I/O, poll readiness, termios/window-size control, canonical input/echo, output processing, terminal signals, session ownership, and foreground process-group checks. Noncanonical `VMIN`/`VTIME` timing and packet-mode PTYs remain unsupported.
- [x] Add generic descriptor-based device-control dispatch for `ioctl`/`posix_devctl` and terminal window-size operations. Terminal attributes and line discipline use the existing TTY bridge.
- [x] Implement unnamed `AF_UNIX` `SOCK_STREAM` `socketpair` endpoints with bounded in-memory duplex queues, nonblocking mode, shutdown/EOF, descriptor duplication, and poll readiness.
- [ ] Add ancillary-message passing (`SCM_RIGHTS`/credentials) and out-of-band local socket support. Pathname `AF_UNIX SOCK_STREAM` sockets now support VFS socket nodes, bind/connect/listen/accept, local name queries, and stream I/O; unnamed `socketpair` remains supported separately.
- [x] Add a kernel cryptographically secure random-byte service and expose it to `getentropy` using the architecture-gated RDRAND/RNDR source.

## Existing Facilities to Wire

- [x] Adapt `select`/`pselect` to descriptor readiness and the existing poll/event interfaces; this is primarily POSIX bridge integration, not a new event engine.
- [x] Implement POSIX `DIR` stream wrappers over VFS directory iteration; the missing work is the descriptor-backed stream contract and libc bridge.
- [x] Integrate `dlopen`/`dlsym`/`dlclose` with the existing ELF loading and symbol machinery in [`runtime/bridge/posix/dlfcn.rs`](runtime/bridge/posix/dlfcn.rs), including dependency loading, relocation scopes, process-scoped opaque handles, and release of unreachable mappings. Relocation is eager for both `RTLD_LAZY` and `RTLD_NOW`; the symbol scope contains loaded dependencies and `RTLD_GLOBAL` objects, not a separately registered main-executable symbol table.
- [x] Connect POSIX timers to the existing event timer queue and runtime scheduling/cancellation hooks in [`runtime/bridge/posix/time.rs`](runtime/bridge/posix/time.rs) and [`runtime/ffi.rs`](runtime/ffi.rs), including `SIGEV_NONE`, `SIGEV_SIGNAL`, `SIGEV_THREAD`, absolute/relative and periodic deadlines, querying, deletion, and overrun counts.

Math, floating-point environment, regex, locale/formatting, account and group databases, DBM, search helpers, and shell-style expansion also have unsupported POSIX entry points, but those are libc or userspace-service work rather than missing kernel components.