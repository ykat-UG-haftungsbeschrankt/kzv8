<!--
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
-->

# Accepted Runtime Behavior

## Address-Derived IDs

The current trusted runtime intentionally exposes address-derived values as opaque IDs:

- Timer IDs identify reusable timer slots and are returned to runtime code for cancellation.
- Readiness event IDs derive from the address of their `EventSignal` allocation and are exposed as `event.id` by the V8 bridge.
- `gettid` returns the address-derived identity of the current thread object.

These values are correlation tokens only. They are not secrets, authorization credentials, or stable identities across object lifetimes. Consumers must not dereference them or rely on their numeric representation.

Timer IDs have no generation component. Once a timer fires or is canceled, its slot can be reused; a stale ID can therefore alias a later timer in that slot. Under the current trusted-runtime contract, callers must discard a timer ID after its timer fires or is canceled and must not use stale IDs. This reuse behavior is accepted for the current implementation.

This is a runtime-specific exception to the general object-identity guidance in `AGENTS.md` and assumes scripts using these interfaces are trusted. The numeric values are opaque IDs, never pointers to dereference. If untrusted scripts or processes are introduced, revisit this decision before exposing them: use opaque generation-checked handles and do not disclose kernel addresses to those callers.

## Kernel Audit Backlog

Work these items from top to bottom. Check an item off only after its behavior is implemented and validated. Build and QEMU validation are currently gated by `AGENTS.md` and need an explicit request.

### P1: Correctness and Isolation

- [ ] **K-01: Per-thread x86 syscall stack.** `SYSCALL` switches to the per-CPU stack pointer in `gs:[0]`, but it is set when entering userspace rather than when a suspended userspace thread is resumed. After a context switch, a syscall may use another thread's kernel stack. Update the per-CPU syscall/TSS stack state whenever scheduling a userspace thread, and cover switching between two suspended syscall contexts. See [x86 syscall entry](kernel/arch/x86_64/syscall.rs) and [thread switching](kernel/scheduler/thread.rs).
- [ ] **K-02: Per-process address spaces.** `Process` has no address-space owner, `UserAddressSpace` is a unit struct, and user mappings are recorded by a global `VirtualAreas`. Separate processes can therefore mutate the same user mapping state. Add an address-space object to `Process`, route mapping/protection operations through it, and switch the architecture page-table root with the scheduled process/thread. See [process state](kernel/process.rs), [user VM](kernel/scheduler/thread/vm.rs), and [virtual memory registry](kernel/memory/virtual.rs).
- [ ] **K-03: Cross-CPU TLB invalidation.** x86 mapping and permission changes use local `invlpg`, while secondary CPUs can run concurrently. A remote CPU may retain stale translations after unmap or permission reduction. Add acknowledged TLB shootdowns before a mapping is reused or its backing memory is released. See [x86 paging](kernel/arch/x86_64/paging.rs) and [SMP startup](kernel/main.rs).
- [ ] **K-04: Block timeout and shared-buffer ownership.** A client timeout releases `request_active` even though the queued worker may still be using the shared-memory grant. A subsequent request can overwrite that buffer. Retain the buffer slot until completion/cancellation is acknowledged, or allocate per-request storage. See [block client](kernel/block/service.rs).
- [ ] **K-05: Bound network memory growth.** TCP send queues, UDP receive queues, ARP neighbors, and unresolved-route queues grow dynamically without per-socket or global byte/entry budgets. Traffic or applications can exhaust kernel memory. Add configurable quotas, backpressure, and explicit drop/eviction policy. See [TCP connections](kernel/net/tcp/connection.rs), [UDP sockets](kernel/net/udp.rs), and [IPv4 stack queues](kernel/net/stack.rs).
- [ ] **K-06: Finish TCP reliability and security.** The custom TCP implementation keeps one outstanding segment and accepts payload only at the exact next receive sequence; out-of-order data is discarded. Initial sequence numbers use a predictable fixed seed/increment. Add unpredictable ISNs, receive reassembly, congestion control, and interoperability tests before treating TCP as general-purpose. See [TCP connection state](kernel/net/tcp/connection.rs).

### P2: Kernel Interfaces and Lifecycle

- [ ] **K-07: Process IDs at the syscall ABI.** `getpid` always returns `1`, so processes cannot be distinguished through the advertised syscall. Define namespace-aware numeric PID/TID values for the ABI; keep internal pointer identity separate. The address-derived `gettid` behavior is currently recorded above as an accepted trusted-runtime exception. See [getpid](kernel/syscall/fn/getpid.rs), [gettid](kernel/syscall/fn/gettid.rs), and [process identity](kernel/process.rs).
- [ ] **K-08: Make namespaces enforce isolation.** Namespace objects currently provide identity/sharing checks, but VFS, network, IPC, and PID lookups do not select namespace-owned resource views. Either connect these subsystems to namespace state or document the namespace API as scaffolding rather than isolation. See [namespace sets](kernel/namespace.rs).
- [ ] **K-09: Connect VFS objects to syscall descriptors.** `Process` has kernel-file-object storage, but no syscall open/read/close dispatch uses it; `write` is restricted to stdout/stderr. Add a per-process descriptor table and wire it to VFS `FileHandle` operations. See [syscall dispatch](kernel/syscall/mod.rs), [write syscall](kernel/syscall/fn/write.rs), [process file objects](kernel/process.rs), and [VFS open](kernel/vfs/mod.rs).
- [ ] **K-10: Make timer IDs generation-safe.** Timer IDs identify reusable timer-slot addresses without a generation. A stale ID can cancel a later timer occupying the same slot. Use an index-plus-generation token at the API boundary. This is separate from the accepted opaque-ID exposure policy above. See [timer IDs and cancellation](kernel/event/timers.rs).
- [ ] **K-11: Make file append atomic.** `FileHandle::write` obtains the current size and then invokes `write_at` separately; concurrent append handles can select the same offset unless every plugin independently serializes both operations. Add an atomic append operation to the plugin/inode contract. See [VFS file writes](kernel/vfs/mod.rs).
- [ ] **K-12: Add TLB/page-table reclamation.** Architecture backends draw page tables from fixed 2048-entry static arrays with monotonically increasing allocation indices. Mapping churn can exhaust those arrays even after mappings are released. Allocate page-table pages from the physical allocator and reclaim empty tables. See [x86 page-table storage](kernel/arch/x86_64/paging.rs) and [AArch64 page-table storage](kernel/arch/aarch64/ram_map.rs).

### P2: Network Scaling and Coverage

- [ ] **K-13: Index hot network lookups.** Capability allocation scans for a free slot, UDP demultiplexing scans every socket, and route selection scans all routes for each packet. Add a capability free list, a keyed UDP/TCP flow table, and a prefix-aware route index when workload size warrants it. See [capability table](kernel/process.rs), [UDP demultiplexing](kernel/net/udp.rs), and [route lookup](kernel/net/stack.rs).
- [ ] **K-14: Remove fixed vSwitch peer/ring capacities.** The VM switch currently supports eight guest peers and eight queued frames per peer; the host frame rings are also fixed-size. Make these limits configurable or dynamically allocated with explicit resource quotas and observable drops. See [VM vSwitch](kernel/vm/device/virtio/net.rs).
- [ ] **K-15: Complete network protocol coverage.** The stack is IPv4-only; host delivery drops fragmented IPv4 datagrams, and IPv6, DHCP/DNS, NAT, and broader raw-socket support are absent. Decide and document the supported protocol contract, then add missing services or return precise unsupported errors. See [IPv4 parsing](kernel/net/wire.rs), [host packet dispatch](kernel/net/stack.rs), and [POSIX sockets](runtime/bridge/posix/sys/socket.rs).
- [ ] **K-16: Unify packet egress and ingress dispatch.** TCP, UDP, ICMP, forwarding, Ethernet, and TUN paths repeat packet assembly, routing, and delivery decisions. Introduce a shared IPv4 egress path and a common ingress dispatch point, retaining protocol-specific serialization and checks. See [network stack](kernel/net/stack.rs).

### P2: Validation Infrastructure

- [ ] **K-17: Make kernel tests buildable and fix stale route tests.** `Ipv4RouteTable::new` requires local address, prefix, and gateway, but its embedded tests call it with no arguments. The kernel binary also has `test = false`, so these tests are not exercised by Cargo's normal test target. Move tests to a testable target or add a dedicated harness, update constructors, and run it when validation is authorized. See [route table/tests](kernel/net/stack.rs) and [Cargo target settings](Cargo.toml).