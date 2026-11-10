/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

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
    bytes: &'a [u8],
}

impl<'a> SliceFile<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl FileSource for SliceFile<'_> {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, Error> {
        let offset = usize::try_from(offset).map_err(|_| Error::Overflow)?;
        if offset >= self.bytes.len() {
            return Ok(0);
        }
        let count = buffer.len().min(self.bytes.len() - offset);
        buffer[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        Ok(count)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Permissions(u8);

impl Permissions {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const READ: Self = Self(1);
    pub(crate) const WRITE: Self = Self(2);
    pub(crate) const EXECUTE: Self = Self(4);

    pub(crate) const fn from_bits(bits: u8) -> Self {
        Self(bits & 0x07)
    }

    pub(crate) const fn bits(self) -> u8 {
        self.0
    }
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
    symbols: Vec<ScopedSymbol<'a>>,
}

impl<'a> SymbolScope<'a> {
    pub(crate) fn new(objects: &[&'a LoadedObject]) -> Result<Self, Error> {
        const MAX_SCOPE_SYMBOLS: usize = 1 << 20;

        let symbol_count = objects.iter().try_fold(0usize, |count, object| {
            count
                .checked_add(object.exported_symbols().count())
                .ok_or(Error::Overflow)
        })?;
        if symbol_count > MAX_SCOPE_SYMBOLS {
            return Err(Error::Fbig);
        }
        let mut symbols = Vec::new();
        symbols
            .try_reserve_exact(symbol_count)
            .map_err(|_| Error::NoMem)?;
        let mut order = 0usize;
        for object in objects {
            for (name, address, _weak) in object.exported_symbols() {
                symbols.push(ScopedSymbol {
                    name,
                    address,
                    order,
                });
                order += 1;
            }
        }
        symbols.sort_unstable_by(|left, right| {
            left.name.cmp(right.name).then(left.order.cmp(&right.order))
        });
        Ok(Self { symbols })
    }
}

impl SymbolResolver for SymbolScope<'_> {
    fn resolve(&mut self, name: &[u8]) -> Option<ResolvedSymbol> {
        let index = self.symbols.partition_point(|symbol| symbol.name < name);
        self.symbols
            .get(index)
            .filter(|symbol| symbol.name == name)
            .map(|symbol| ResolvedSymbol {
                address: symbol.address,
            })
    }
}