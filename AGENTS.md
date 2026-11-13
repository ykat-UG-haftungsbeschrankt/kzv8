## Read-Only Files
- Never modify a file containing the `//@READONLY` comment. This includes editing, reformatting, regeneration, and automated transformations. If a task requires changing such a file, leave it untouched and explain the constraint.
- When working on files under `runtime/`, do not modify files under `kernel/`. If a runtime task appears to require a kernel change, explain why and ask for approval before editing `kernel/`.
- For POSIX interfaces under `runtime/bridge/posix`, do not remove or change existing functions or constants, including function names and signatures or constant names and values. Change an existing POSIX function only by updating its body, and only when required to accommodate a kernel change; preserve its POSIX-facing semantics.

## Copyright Headers
- Add the following notice to every new or modified first-party text source file. Preserve its wording and use the file's native comment syntax.
```c
/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */
```
- Do not add comments to formats that do not support them, or to generated, vendored, or binary files.

## Validation
- Run code reformatting, compilation/builds, and smoke checks only when explicitly requested by the user.

## Kernel Testing Layout
- Put kernel unit tests and test-only helpers in `tests/kernel/`, mirroring the tested module's path (for example, `kernel/net/tcp/connection.rs` uses `tests/kernel/net/tcp/connection.rs`). Do not add inline `#[test]` bodies to files under `kernel/`.
- Keep only a minimal `#[cfg(test)]` path-module hook in the owning kernel module so Cargo discovers the tests as descendants of that module and their existing private access is preserved.
- When adding, moving, splitting, or renaming a kernel module, update its mirrored test file and owner-side path hook together. When changing module behavior or API, update or add the corresponding tests in the same change so implementation and coverage remain synchronized.
- Keep other kernel-side `#[cfg(test)]` code only when structurally required by the test build, such as test stubs, architecture shims, or test-only state. Keep test cases and test-specific helper logic in `tests/kernel/`.
- Keep Cargo unit tests on the Cargo test harness; do not route them through `runtime/run.rs`. Runtime smoke checks are separate from unit tests.

## Kernel Errors
- Define all kernel error codes in `kernel/error.rs`; do not define kernel error codes elsewhere.
- Kernel Functions that report errors must return an appropriate code from `kernel/error.rs`; do not introduce module-local error enums or numeric error codes.

## Application TypeScript
- Use TypeScript for application source files under `app/`; do not add JavaScript source files there.
- This rule does not apply under `app/build/` or `app/node_modules/`.

## Rust Safety
- Prefer safe Rust and avoid `unsafe` whenever a safe API can express the operation.
- When `unsafe` is necessary, keep the block minimal and document its safety invariants.

## API and Implementation Separation
- Keep module root files such as `mod.rs` focused on the module's API: type and trait declarations, constants, and re-exports.
- Put concrete implementations, private state, and implementation-only helpers in child modules. Implement API traits outside the module root.
- When splitting an existing module, preserve established import paths with re-exports. Use this separation where it clarifies a meaningful API boundary; avoid extra modules for trivial code.

## Object Identity
- For identity of a live in-memory object, use its address (for `Arc<T>`, `Arc::as_ptr`) rather than assigning a separate integer ID and maintaining an integer-to-object mapping.
- A trusted userspace API may expose that address as an opaque identity or correlation token for a live object. Do not dereference it, and do not use a raw kernel address as a descriptor, authorization token, or handle accepted back from userspace.
- Treat pointer-derived values as opaque identity only; never dereference an address received from userspace or another untrusted boundary. Keep validated generational handles where an external ABI or stale-handle protection requires them.
- `TimerQueue` and `EventLoop` keep timers as `Arc<Timer>` internally; cancellation operates on that `Arc`. At the runtime FFI boundary, each timer ID is a `KernelObjectRef` wrapping the `Arc::as_ptr` address of a heap-allocated `Timer`, with no timer-specific ID wrapper or generation field. To cancel, the FFI extracts the reference's address and resolves it by searching the deadline tree; an ID received from runtime code is never dereferenced. IDs are invalid after cancellation or expiry and callers must discard them. Once the timer's `Arc` allocation is freed, address reuse can make a stale ID alias a later timer. Keep this address-only policy only for the current trusted runtime contract; add a generation-checked token before exposing timer IDs across an untrusted boundary.

## Rust Naming
Follow the [Rust API Guidelines naming conventions](https://rust-lang.github.io/api-guidelines/naming.html):
- Use `snake_case` for modules, functions, methods, macros, and local variables; use `UpperCamelCase` for types, traits, and enum variants; use `SCREAMING_SNAKE_CASE` for constants and statics.
- Keep type parameters concise and `UpperCamelCase`; use short lowercase lifetime names. Treat acronyms as words, such as `Uuid` and `is_xid_start`.
- Use `new` or `with_*` for constructors. Name conversions `as_*` for cheap borrowed views, `to_*` for conversions that may do work or allocate, and `into_*` when consuming the input.
- Prefer getter names like `name()` and `name_mut()` over `get_name()`, except for conventional single-value `get()` APIs. Name collection iterators `iter`, `iter_mut`, and `into_iter`, with corresponding iterator types.
- Use descriptive Cargo feature names without placeholder prefixes such as `use-` or `with-`; avoid negative feature names and keep compound-name word order consistent.
- Do not add `-rs` or `-rust` to crate names.
