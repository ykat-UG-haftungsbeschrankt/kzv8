/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod device;
mod filesystem;

#[allow(unused_imports)]
pub(crate) use filesystem::VmFileSystem;

#[cfg(test)]
#[path = "../../../../tests/kernel/vfs/filesystem/vm.rs"]
mod tests;/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod device;