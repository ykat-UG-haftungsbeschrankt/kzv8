When working on files under `runtime/`, do not modify files under `kernel/`. If a runtime task appears to require a kernel change, explain why and ask for approval before editing `kernel/`.

## Kernel Errors
- Define all kernel error codes in `kernel/error.rs`; do not define kernel error codes elsewhere.

## Rust Safety
- Prefer safe Rust and avoid `unsafe` whenever a safe API can express the operation.
- When `unsafe` is necessary, keep the block minimal and document its safety invariants.

## Rust Naming
Follow the [Rust API Guidelines naming conventions](https://rust-lang.github.io/api-guidelines/naming.html):
- Use `snake_case` for modules, functions, methods, macros, and local variables; use `UpperCamelCase` for types, traits, and enum variants; use `SCREAMING_SNAKE_CASE` for constants and statics.
- Keep type parameters concise and `UpperCamelCase`; use short lowercase lifetime names. Treat acronyms as words, such as `Uuid` and `is_xid_start`.
- Use `new` or `with_*` for constructors. Name conversions `as_*` for cheap borrowed views, `to_*` for conversions that may do work or allocate, and `into_*` when consuming the input.
- Prefer getter names like `name()` and `name_mut()` over `get_name()`, except for conventional single-value `get()` APIs. Name collection iterators `iter`, `iter_mut`, and `into_iter`, with corresponding iterator types.
- Use descriptive Cargo feature names without placeholder prefixes such as `use-` or `with-`; avoid negative feature names and keep compound-name word order consistent.
- Do not add `-rs` or `-rust` to crate names.
