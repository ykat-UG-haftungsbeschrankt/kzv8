/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use super::{
    Error, FileSource, LoadedObject, Permissions, PermissionsApi, ResolvedSymbol, SliceFile,
    SliceFileApi, SymbolResolver, SymbolScope, SymbolScopeApi,
};

pub(super) struct SliceFileState<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceFileApi<'a> for SliceFile<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            state: SliceFileState { bytes },
        }
    }
}

impl FileSource for SliceFile<'_> {
    fn len(&self) -> u64 {
        self.state.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, Error> {
        let offset = usize::try_from(offset).map_err(|_| Error::Overflow)?;
        if offset >= self.state.bytes.len() {
            return Ok(0);
        }
        let count = buffer.len().min(self.state.bytes.len() - offset);
        buffer[..count].copy_from_slice(&self.state.bytes[offset..offset + count]);
        Ok(count)
    }
}

impl PermissionsApi for Permissions {
    const NONE: Self = Self(0);
    const READ: Self = Self(1);
    const WRITE: Self = Self(2);
    const EXECUTE: Self = Self(4);

    fn from_bits(bits: u8) -> Self {
        Self(bits & 0x07)
    }

    fn bits(self) -> u8 {
        self.0
    }
}

struct ScopedSymbol<'a> {
    name: &'a [u8],
    address: usize,
    order: usize,
}

pub(super) struct SymbolScopeState<'a> {
    symbols: Vec<ScopedSymbol<'a>>,
}

impl<'a> SymbolScopeApi<'a> for SymbolScope<'a> {
    fn new(objects: &[&'a LoadedObject]) -> Result<Self, Error> {
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
        Ok(Self {
            state: SymbolScopeState { symbols },
        })
    }
}

impl SymbolResolver for SymbolScope<'_> {
    fn resolve(&mut self, name: &[u8]) -> Option<ResolvedSymbol> {
        let symbols = &self.state.symbols;
        let index = symbols.partition_point(|symbol| symbol.name < name);
        symbols
            .get(index)
            .filter(|symbol| symbol.name == name)
            .map(|symbol| ResolvedSymbol {
                address: symbol.address,
            })
    }
}