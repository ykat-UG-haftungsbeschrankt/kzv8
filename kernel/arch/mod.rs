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

pub(crate) trait MemoryArchitecture {
    fn initialize_mmu();
    fn is_identity_mapped(address: usize) -> bool;
    fn protect_kernel_image_pages(address: usize, length: usize, permissions: i32) -> bool;
    fn set_page_permissions(address: usize, length: usize, permissions: i32) -> bool;
    fn map_page(virtual_address: usize, physical_address: usize, permissions: i32) -> bool;
    fn unmap_page(virtual_address: usize) -> Option<usize>;
    fn page_is_mapped(virtual_address: usize) -> bool;
    fn page_has_backing(virtual_address: usize) -> bool;
}

#[cfg(target_arch = "aarch64")]
#[path = "aarch64/mod.rs"]
mod arch;
#[cfg(target_arch = "x86_64")]
#[path = "x86_64/mod.rs"]
mod arch;

pub(crate) use arch::*;
pub(crate) use arch::MemoryArch as ActiveMemoryArch;
