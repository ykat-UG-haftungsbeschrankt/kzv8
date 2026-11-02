/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VirtioWindow {
    pub(crate) start: usize,
    pub(crate) slots: usize,
    pub(crate) stride: usize,
}

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/mod.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/mod.rs"]
mod arch;

pub(crate) use arch::*;
