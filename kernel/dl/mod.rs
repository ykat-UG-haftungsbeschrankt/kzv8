/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

mod dl;
mod elf;

pub(crate) use elf::{ElfKind, LoadedObject};

pub(crate) use crate::kernel::error::Error;

pub(crate) trait FileSource {
    /// Return the exact byte length of this random-access source.
    ///
    /// The length and bytes returned by this source must remain unchanged for the
    /// duration of `LoadedObject::load`. Mutable backing stores should provide a snapshot.
    fn len(&self) -> u64;
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, Error>;
}

pub(crate) struct SliceFile<'a> {
    state: dl::SliceFileState<'a>,
}

pub(crate) trait SliceFileApi<'a>: Sized {
    fn new(bytes: &'a [u8]) -> Self;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Permissions(u8);

pub(crate) trait PermissionsApi: Sized {
    const NONE: Self;
    const READ: Self;
    const WRITE: Self;
    const EXECUTE: Self;

    fn from_bits(bits: u8) -> Self;
    fn bits(self) -> u8;
}

pub(crate) trait AddressSpace {
    /// Reserve a zeroable image span for temporary writable, non-executable mappings.
    /// A fixed request must be honored exactly; otherwise the returned address must
    /// be congruent to `alignment_offset` modulo `alignment`.
    fn reserve(
        &mut self,
        preferred_address: Option<usize>,
        length: usize,
        alignment: usize,
        alignment_offset: usize,
        fixed: bool,
    ) -> Result<usize, Error>;

    fn zero(&mut self, address: usize, length: usize) -> Result<(), Error>;
    fn read(&self, address: usize, buffer: &mut [u8]) -> Result<(), Error>;
    fn write(&mut self, address: usize, buffer: &[u8]) -> Result<(), Error>;
    /// Apply final page permissions after relocations have completed.
    fn protect(
        &mut self,
        address: usize,
        length: usize,
        permissions: Permissions,
    ) -> Result<(), Error>;
    fn release(&mut self, address: usize, length: usize);
}

pub(crate) trait SymbolResolver {
    fn resolve(&mut self, name: &[u8]) -> Option<ResolvedSymbol>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedSymbol {
    pub(crate) address: usize,
}

struct ScopedSymbol<'a> {
    name: &'a [u8],
    address: usize,
    order: usize,
}

pub(crate) struct SymbolScope<'a> {
    state: dl::SymbolScopeState<'a>,
}

pub(crate) trait SymbolScopeApi<'a>: Sized {
    fn new(objects: &[&'a LoadedObject]) -> Result<Self, Error>;
}