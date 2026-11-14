/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use crate::kernel::lib::sorted_vector::SortedVector;

use super::{
    AddressSpace, Error, FileSource, LoadedObject, ObjectKind, Permissions, PermissionsApi,
    ResolvedSymbol, SliceFile, SliceFileApi, SymbolResolver, SymbolScope, SymbolScopeApi,
};

pub(super) const PAGE_SIZE: usize = 4096;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ObjectState {
    RelocationPending,
    ReadyToSeal,
    Sealed,
    Failed,
}

pub(super) enum LoadedObjectState {
    Elf(super::elf::ElfLoadedObject),
    Pe(super::pe::PeLoadedObject),
    MachO(super::mach_o::MachOLoadedObject),
}

impl LoadedObject {
    pub(crate) fn load<F: FileSource, A: AddressSpace>(
        source: &F,
        address_space: &mut A,
        kind: ObjectKind,
        preferred_address: Option<usize>,
    ) -> Result<Self, Error> {
        if source.len() < 4 {
            return Err(Error::NoExec);
        }
        let mut magic = [0u8; 4];
        read_exact(source, 0, &mut magic)?;
        let state = if super::elf::matches(&magic) {
            LoadedObjectState::Elf(super::elf::ElfLoadedObject::load(
                source,
                address_space,
                kind,
                preferred_address,
            )?)
        } else if super::pe::matches(&magic) {
            super::pe::load(source, address_space, kind, preferred_address)?
        } else if super::mach_o::matches(&magic) {
            super::mach_o::load(source, address_space, kind, preferred_address)?
        } else {
            return Err(Error::NoExec);
        };
        Ok(Self { state })
    }

    pub(crate) const fn kind(&self) -> ObjectKind {
        match &self.state {
            LoadedObjectState::Elf(object) => object.kind(),
            LoadedObjectState::Pe(object) => object.kind(),
            LoadedObjectState::MachO(object) => object.kind(),
        }
    }

    pub(crate) const fn mapping_base(&self) -> usize {
        match &self.state {
            LoadedObjectState::Elf(object) => object.mapping_base(),
            LoadedObjectState::Pe(object) => object.mapping_base(),
            LoadedObjectState::MachO(object) => object.mapping_base(),
        }
    }

    pub(crate) const fn mapping_length(&self) -> usize {
        match &self.state {
            LoadedObjectState::Elf(object) => object.mapping_length(),
            LoadedObjectState::Pe(object) => object.mapping_length(),
            LoadedObjectState::MachO(object) => object.mapping_length(),
        }
    }

    pub(crate) const fn load_bias(&self) -> i128 {
        match &self.state {
            LoadedObjectState::Elf(object) => object.load_bias(),
            LoadedObjectState::Pe(object) => object.load_bias(),
            LoadedObjectState::MachO(object) => object.load_bias(),
        }
    }

    pub(crate) const fn entry(&self) -> Option<usize> {
        match &self.state {
            LoadedObjectState::Elf(object) => object.entry(),
            LoadedObjectState::Pe(object) => object.entry(),
            LoadedObjectState::MachO(object) => object.entry(),
        }
    }

    pub(crate) const fn is_sealed(&self) -> bool {
        match &self.state {
            LoadedObjectState::Elf(object) => object.is_sealed(),
            LoadedObjectState::Pe(object) => object.is_sealed(),
            LoadedObjectState::MachO(object) => object.is_sealed(),
        }
    }

    pub(crate) fn needed(&self) -> &[Vec<u8>] {
        match &self.state {
            LoadedObjectState::Elf(object) => object.needed(),
            LoadedObjectState::Pe(object) => object.needed(),
            LoadedObjectState::MachO(object) => object.needed(),
        }
    }

    pub(crate) fn search_paths(&self) -> &[Vec<u8>] {
        match &self.state {
            LoadedObjectState::Elf(_) | LoadedObjectState::Pe(_) => &[],
            LoadedObjectState::MachO(object) => object.search_paths(),
        }
    }

    pub(crate) fn dependency_is_weak(&self, index: usize) -> bool {
        match &self.state {
            LoadedObjectState::Elf(_) | LoadedObjectState::Pe(_) => false,
            LoadedObjectState::MachO(object) => object.dependency_is_weak(index),
        }
    }

    pub(crate) fn symbol(&self, name: &[u8]) -> Option<usize> {
        match &self.state {
            LoadedObjectState::Elf(object) => object.symbol(name),
            LoadedObjectState::Pe(object) => object.symbol(name),
            LoadedObjectState::MachO(object) => object.symbol(name),
        }
    }

    pub(crate) fn symbol_size(&self, name: &[u8]) -> Option<u64> {
        match &self.state {
            LoadedObjectState::Elf(object) => object.symbol_size(name),
            LoadedObjectState::Pe(object) => object.symbol_size(name),
            LoadedObjectState::MachO(object) => object.symbol_size(name),
        }
    }

    pub(super) fn symbol_ordinal(&self, ordinal: u32) -> Option<usize> {
        match &self.state {
            LoadedObjectState::Elf(_) => None,
            LoadedObjectState::Pe(object) => object.symbol_ordinal(ordinal),
            LoadedObjectState::MachO(_) => None,
        }
    }

    pub(super) fn for_each_exported_symbol<'a>(
        &'a self,
        visitor: &mut dyn FnMut(&'a [u8], usize, bool),
    ) {
        match &self.state {
            LoadedObjectState::Elf(object) => object.for_each_exported_symbol(visitor),
            LoadedObjectState::Pe(object) => object.for_each_exported_symbol(visitor),
            LoadedObjectState::MachO(object) => object.for_each_exported_symbol(visitor),
        }
    }

    pub(crate) fn relocate<A: AddressSpace, R: SymbolResolver>(
        &mut self,
        address_space: &mut A,
        resolver: &mut R,
    ) -> Result<(), Error> {
        match &mut self.state {
            LoadedObjectState::Elf(object) => object.relocate(address_space, resolver),
            LoadedObjectState::Pe(object) => object.relocate(address_space, resolver),
            LoadedObjectState::MachO(object) => object.relocate(address_space, resolver),
        }
    }

    pub(crate) fn seal<A: AddressSpace>(
        &mut self,
        address_space: &mut A,
    ) -> Result<(), Error> {
        match &mut self.state {
            LoadedObjectState::Elf(object) => object.seal(address_space),
            LoadedObjectState::Pe(object) => object.seal(address_space),
            LoadedObjectState::MachO(object) => object.seal(address_space),
        }
    }

    pub(crate) fn release<A: AddressSpace>(self, address_space: &mut A) {
        match self.state {
            LoadedObjectState::Elf(object) => object.release(address_space),
            LoadedObjectState::Pe(object) => object.release(address_space),
            LoadedObjectState::MachO(object) => object.release(address_space),
        }
    }
}

pub(super) fn copy_file_range<F: FileSource, A: AddressSpace>(
    source: &F,
    address_space: &mut A,
    file_offset: u64,
    target_address: usize,
    length: u64,
) -> Result<(), Error> {
    let mut buffer = [0u8; 1024];
    let mut copied = 0u64;
    while copied < length {
        let count = usize::try_from((length - copied).min(buffer.len() as u64))
            .map_err(|_| Error::Overflow)?;
        let offset = file_offset.checked_add(copied).ok_or(Error::Overflow)?;
        read_exact(source, offset, &mut buffer[..count])?;
        let address = target_address
            .checked_add(usize::try_from(copied).map_err(|_| Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        address_space.write(address, &buffer[..count])?;
        copied += count as u64;
    }
    Ok(())
}

pub(super) fn read_exact<F: FileSource>(
    source: &F,
    offset: u64,
    buffer: &mut [u8],
) -> Result<(), Error> {
    let mut read = 0usize;
    while read < buffer.len() {
        let current_offset = offset
            .checked_add(read as u64)
            .ok_or(Error::Overflow)?;
        let count = source.read_at(current_offset, &mut buffer[read..])?;
        if count == 0 || count > buffer.len() - read {
            return Err(Error::Io);
        }
        read += count;
    }
    Ok(())
}

pub(super) fn protect_pages<A: AddressSpace>(
    state: &mut ObjectState,
    address_space: &mut A,
    mapping_base: usize,
    mapping_length: usize,
    page_permissions: &[u8],
) -> Result<(), Error> {
    if *state != ObjectState::ReadyToSeal || mapping_length % PAGE_SIZE != 0 {
        return Err(Error::Inval);
    }
    let page_count = mapping_length / PAGE_SIZE;
    if page_permissions.len() != page_count {
        return Err(Error::Inval);
    }

    let mut page_index = 0usize;
    while page_index < page_count {
        let permissions = page_permissions[page_index];
        if permissions & (Permissions::WRITE.bits() | Permissions::EXECUTE.bits())
            == (Permissions::WRITE.bits() | Permissions::EXECUTE.bits())
        {
            *state = ObjectState::Failed;
            return Err(Error::OpNotSupp);
        }
        let first_page = page_index;
        page_index += 1;
        while page_index < page_count && page_permissions[page_index] == permissions {
            page_index += 1;
        }
        let address = mapping_base
            .checked_add(first_page.checked_mul(PAGE_SIZE).ok_or(Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        let length = (page_index - first_page)
            .checked_mul(PAGE_SIZE)
            .ok_or(Error::Overflow)?;
        if let Err(error) = address_space.protect(
            address,
            length,
            Permissions::from_bits(permissions),
        ) {
            *state = ObjectState::Failed;
            return Err(error);
        }
    }
    *state = ObjectState::Sealed;
    Ok(())
}

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
    weak: bool,
}

struct ScopedDependency<'a> {
    name: &'a [u8],
    object: &'a LoadedObject,
}

pub(super) struct SymbolScopeState<'a> {
    //@FIXME: INDEXTYPE(symbols); SortedVector preserves name/strength/order sorting with O(log n) lower-bound lookup.
    symbols: SortedVector<ScopedSymbol<'a>>,
    //@FIXME: INDEXTYPE(dependencies); ordered first-match lookup scans O(n); add a hash side index for expected O(1) lookup if dependency counts grow.
    dependencies: Vec<ScopedDependency<'a>>,
}

impl<'a> SymbolScopeApi<'a> for SymbolScope<'a> {
    fn new(objects: &[&'a LoadedObject]) -> Result<Self, Error> {
        Self::build(objects, &[])
    }

    fn with_dependencies(
        objects: &[&'a LoadedObject],
        dependencies: &[(&'a [u8], &'a LoadedObject)],
    ) -> Result<Self, Error> {
        Self::build(objects, dependencies)
    }
}

impl<'a> SymbolScope<'a> {
    fn build(
        objects: &[&'a LoadedObject],
        dependencies: &[(&'a [u8], &'a LoadedObject)],
    ) -> Result<Self, Error> {
        const MAX_SCOPE_SYMBOLS: usize = 1 << 20;

        let mut symbol_count = 0usize;
        for object in objects {
            let mut overflow = false;
            object.for_each_exported_symbol(&mut |_, _, _| {
                if let Some(next) = symbol_count.checked_add(1) {
                    symbol_count = next;
                } else {
                    overflow = true;
                }
            });
            if overflow {
                return Err(Error::Overflow);
            }
        }
        if symbol_count > MAX_SCOPE_SYMBOLS {
            return Err(Error::Fbig);
        }
        let mut symbols = Vec::new();
        symbols
            .try_reserve_exact(symbol_count)
            .map_err(|_| Error::NoMem)?;
        let mut order = 0usize;
        for object in objects {
            object.for_each_exported_symbol(&mut |name, address, weak| {
                symbols.push(ScopedSymbol {
                    name,
                    address,
                    order,
                    weak,
                });
                order += 1;
            });
        }
        let symbols = SortedVector::from_vec_by(symbols, |left, right| {
            left.name
                .cmp(right.name)
                .then(left.weak.cmp(&right.weak))
                .then(left.order.cmp(&right.order))
        });
        let mut scoped_dependencies = Vec::new();
        scoped_dependencies
            .try_reserve_exact(dependencies.len())
            .map_err(|_| Error::NoMem)?;
        scoped_dependencies.extend(dependencies.iter().map(|(name, object)| {
            ScopedDependency {
                name,
                object,
            }
        }));
        Ok(Self {
            state: SymbolScopeState {
                symbols,
                dependencies: scoped_dependencies,
            },
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

    fn resolve_in(&mut self, library: &[u8], name: &[u8]) -> Option<ResolvedSymbol> {
        let object = self
            .state
            .dependencies
            .iter()
            .find(|dependency| dependency.name == library)?
            .object;
        object.symbol(name).map(|address| ResolvedSymbol { address })
    }

    fn resolve_ordinal_in(&mut self, library: &[u8], ordinal: u32) -> Option<ResolvedSymbol> {
        let object = self
            .state
            .dependencies
            .iter()
            .find(|dependency| dependency.name == library)?
            .object;
        object
            .symbol_ordinal(ordinal)
            .map(|address| ResolvedSymbol { address })
    }
}