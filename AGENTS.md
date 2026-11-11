When working on files under `runtime/`, do not modify files under `kernel/`. If a runtime task appears to require a kernel change, explain why and ask for approval before editing `kernel/`.
For POSIX interfaces under `runtime/bridge/posix`, do not remove or change existing functions or constants, including function names and signatures or constant names and values. Change an existing POSIX function only by updating its body, and only when required to accommodate a kernel change; preserve its POSIX-facing semantics.

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

## Kernel Errors
- Define all kernel error codes in `kernel/error.rs`; do not define kernel error codes elsewhere.
- Kernel Functions that report errors must return an appropriate code from `kernel/error.rs`; do not introduce module-local error enums or numeric error codes.

## Application TypeScript
- Use TypeScript for application source files under `app/`; do not add JavaScript source files there.
- This rule does not apply under `app/build/` or `app/node_modules/`.

## Rust Safety
- Prefer safe Rust and avoid `unsafe` whenever a safe API can express the operation.
- When `unsafe` is necessary, keep the block minimal and document its safety invariants.

## Object Identity
- For identity of a live in-memory object, use its address (for `Arc<T>`, `Arc::as_ptr`) rather than assigning a separate integer ID and maintaining an integer-to-object mapping.
- A trusted userspace API may expose that address as an opaque identity or correlation token for a live object. Do not dereference it, and do not use a raw kernel address as a descriptor, authorization token, or handle accepted back from userspace.
- Treat pointer-derived values as opaque identity only; never dereference an address received from userspace or another untrusted boundary. Keep validated generational handles where an external ABI or stale-handle protection requires them.

## Rust Naming
Follow the [Rust API Guidelines naming conventions](https://rust-lang.github.io/api-guidelines/naming.html):
- Use `snake_case` for modules, functions, methods, macros, and local variables; use `UpperCamelCase` for types, traits, and enum variants; use `SCREAMING_SNAKE_CASE` for constants and statics.
- Keep type parameters concise and `UpperCamelCase`; use short lowercase lifetime names. Treat acronyms as words, such as `Uuid` and `is_xid_start`.
- Use `new` or `with_*` for constructors. Name conversions `as_*` for cheap borrowed views, `to_*` for conversions that may do work or allocate, and `into_*` when consuming the input.
- Prefer getter names like `name()` and `name_mut()` over `get_name()`, except for conventional single-value `get()` APIs. Name collection iterators `iter`, `iter_mut`, and `into_iter`, with corresponding iterator types.
- Use descriptive Cargo feature names without placeholder prefixes such as `use-` or `with-`; avoid negative feature names and keep compound-name word order consistent.
- Do not add `-rs` or `-rust` to crate names.
