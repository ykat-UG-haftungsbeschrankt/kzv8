/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::lib::alignment::align_up;
use crate::kernel::lib::endian;
use super::dl::{copy_file_range, read_exact, ObjectState, PAGE_SIZE};
use super::{
    AddressSpace, Error, FileSource, ObjectKind, Permissions, PermissionsApi as _, SymbolResolver,
};

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const MAX_PROGRAM_HEADERS: usize = 4096;
const MAX_IMAGE_SIZE: usize = 1 << 28;
const MAX_IMAGE_ALIGNMENT: usize = 1 << 30;

const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_TLS: u32 = 7;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const PF_EXECUTE: u32 = 1;
const PF_WRITE: u32 = 2;
const PF_READ: u32 = 4;
const DYNAMIC_ENTRY_SIZE: usize = 16;
const SYMBOL_ENTRY_SIZE: usize = 24;
const MAX_DYNAMIC_ENTRIES: usize = 65_536;
const MAX_DYNAMIC_SYMBOLS: usize = 1 << 17;
const MAX_DYNAMIC_RELOCATIONS: usize = 1 << 18;
const MAX_STRING_TABLE_SIZE: u64 = 16 << 20;
const MAX_SYMBOL_NAME_SIZE: usize = 4096;

const DT_NULL: i64 = 0;
const DT_NEEDED: i64 = 1;
const DT_PLTRELSZ: i64 = 2;
const DT_HASH: i64 = 4;
const DT_STRTAB: i64 = 5;
const DT_SYMTAB: i64 = 6;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;
const DT_RELAENT: i64 = 9;
const DT_STRSZ: i64 = 10;
const DT_SYMENT: i64 = 11;
const DT_REL: i64 = 17;
const DT_RELSZ: i64 = 18;
const DT_RELENT: i64 = 19;
const DT_SYMBOLIC: i64 = 16;
const DT_PLTREL: i64 = 20;
const DT_TEXTREL: i64 = 22;
const DT_JMPREL: i64 = 23;
const DT_FLAGS: i64 = 30;
const DT_RELR: i64 = 36;
const DT_RELRSZ: i64 = 35;
const DT_RELRENT: i64 = 37;
const DT_GNU_HASH: i64 = 0x6fff_fef5;
const DT_VERSYM: i64 = 0x6fff_fff0;
const DT_VERDEF: i64 = 0x6fff_fffc;
const DT_VERDEFNUM: i64 = 0x6fff_fffd;
const DT_VERNEED: i64 = 0x6fff_fffe;
const DT_VERNEEDNUM: i64 = 0x6fff_ffff;
const DF_TEXTREL: u64 = 4;
const DF_SYMBOLIC: u64 = 2;

const SHN_UNDEFINED: u16 = 0;
const SHN_LORESERVE: u16 = 0xff00;
const SHN_ABSOLUTE: u16 = 0xfff1;
const STB_GLOBAL: u8 = 1;
const STB_WEAK: u8 = 2;
const STV_DEFAULT: u8 = 0;
const STV_PROTECTED: u8 = 3;
const STT_TLS: u8 = 6;
const STT_GNU_IFUNC: u8 = 10;
const R_NONE: u32 = 0;

pub(super) fn matches(magic: &[u8; 4]) -> bool {
    *magic == [0x7f, b'E', b'L', b'F']
}

struct DynamicInfo {
    tags: Vec<(i64, u64)>,
    needed_offsets: Vec<u64>,
}

struct DynamicSymbol {
    name: Vec<u8>,
    address: Option<usize>,
    size: u64,
    section_index: u16,
    binding: u8,
    visibility: u8,
    symbol_type: u8,
}

fn is_exported(symbol: &DynamicSymbol) -> bool {
    matches!(symbol.binding, STB_GLOBAL | STB_WEAK)
        && matches!(symbol.visibility, STV_DEFAULT | STV_PROTECTED)
        && !matches!(symbol.symbol_type, STT_TLS | STT_GNU_IFUNC)
}

#[derive(Clone, Copy)]
struct Relocation {
    offset: u64,
    symbol_index: u32,
    kind: u32,
    addend: Option<i64>,
}

#[derive(Clone, Copy)]
struct ProgramHeader {
    kind: u32,
    flags: u32,
    offset: u64,
    virtual_address: u64,
    file_size: u64,
    memory_size: u64,
    alignment: u64,
}

#[derive(Clone, Copy)]
struct LoadRange {
    start: u64,
    end: u64,
    permissions: u32,
}

#[derive(Clone, Copy)]
struct FileRange {
    start: u64,
    end: u64,
    file_offset: u64,
}

struct ParsedElf<'a, F> {
    source: &'a F,
    kind: ObjectKind,
    fixed_address: bool,
    entry: u64,
    program_headers: Vec<ProgramHeader>,
    load_ranges: Vec<LoadRange>,
    file_ranges: Vec<FileRange>,
    minimum_page: usize,
    mapping_length: usize,
    alignment: usize,
}

pub(super) struct ElfLoadedObject {
    kind: ObjectKind,
    mapping_base: usize,
    mapping_length: usize,
    load_bias: usize,
    entry: Option<usize>,
    load_ranges: Vec<LoadRange>,
    relro_segments: Vec<ProgramHeader>,
    needed: Vec<Vec<u8>>,
    symbols: Vec<DynamicSymbol>,
    relocations: Vec<Relocation>,
    state: ObjectState,
}

impl ElfLoadedObject {
    pub(crate) fn load<F: FileSource, A: AddressSpace>(
        source: &F,
        address_space: &mut A,
        kind: ObjectKind,
        preferred_address: Option<usize>,
    ) -> Result<Self, Error> {
        let parsed = ParsedElf::parse(source, kind)?;
        let fixed = parsed.fixed_address;
        let preferred_address = if fixed {
            if preferred_address.is_some_and(|address| address != parsed.minimum_page) {
                return Err(Error::NoExec);
            }
            Some(parsed.minimum_page)
        } else {
            preferred_address
        };
        let mapping_base = address_space.reserve(
            preferred_address,
            parsed.mapping_length,
            parsed.alignment,
            parsed.minimum_page % parsed.alignment,
            fixed,
        )?;
        let Some(load_bias) = mapping_base.checked_sub(parsed.minimum_page) else {
            address_space.release(mapping_base, parsed.mapping_length);
            return Err(Error::NoMem);
        };
        if mapping_base % parsed.alignment != parsed.minimum_page % parsed.alignment
            || (fixed && mapping_base != parsed.minimum_page)
            || mapping_base.checked_add(parsed.mapping_length).is_none()
        {
            address_space.release(mapping_base, parsed.mapping_length);
            return Err(Error::NoMem);
        }

        let result = Self::load_reserved(&parsed, address_space, mapping_base, load_bias);
        if result.is_err() {
            address_space.release(mapping_base, parsed.mapping_length);
        }
        result
    }

    pub(crate) const fn kind(&self) -> ObjectKind {
        self.kind
    }

    pub(crate) const fn mapping_base(&self) -> usize {
        self.mapping_base
    }

    pub(crate) const fn mapping_length(&self) -> usize {
        self.mapping_length
    }

    pub(crate) const fn load_bias(&self) -> i128 {
        self.load_bias as i128
    }

    pub(crate) const fn entry(&self) -> Option<usize> {
        self.entry
    }

    pub(crate) const fn is_sealed(&self) -> bool {
        matches!(self.state, ObjectState::Sealed)
    }

    pub(crate) fn needed(&self) -> &[Vec<u8>] {
        &self.needed
    }

    pub(crate) fn symbol(&self, name: &[u8]) -> Option<usize> {
        self.symbols
            .iter()
            .find(|symbol| {
                symbol.name == name
                    && symbol.address.is_some()
                    && is_exported(symbol)
                    && symbol.binding == STB_GLOBAL
            })
            .or_else(|| {
                self.symbols.iter().find(|symbol| {
                    symbol.name == name
                        && symbol.address.is_some()
                        && is_exported(symbol)
                        && symbol.binding == STB_WEAK
                })
            })
            .and_then(|symbol| symbol.address)
    }

    pub(crate) fn symbol_size(&self, name: &[u8]) -> Option<u64> {
        self.symbols
            .iter()
            .find(|symbol| {
                symbol.name == name
                    && symbol.address.is_some()
                    && is_exported(symbol)
                    && symbol.binding == STB_GLOBAL
            })
            .or_else(|| {
                self.symbols.iter().find(|symbol| {
                    symbol.name == name
                        && symbol.address.is_some()
                        && is_exported(symbol)
                        && symbol.binding == STB_WEAK
                })
            })
            .map(|symbol| symbol.size)
    }

    pub(super) fn for_each_exported_symbol<'a>(
        &'a self,
        visitor: &mut dyn FnMut(&'a [u8], usize, bool),
    ) {
        for symbol in &self.symbols {
            if is_exported(symbol) {
                if let Some(address) = symbol.address {
                    visitor(
                        &symbol.name,
                        address,
                        symbol.binding == STB_WEAK,
                    );
                }
            }
        }
    }

    pub(crate) fn relocate<A: AddressSpace, R: SymbolResolver>(
        &mut self,
        address_space: &mut A,
        resolver: &mut R,
    ) -> Result<(), Error> {
        match self.state {
            ObjectState::RelocationPending => {}
            ObjectState::ReadyToSeal => return Ok(()),
            ObjectState::Sealed | ObjectState::Failed => return Err(Error::Inval),
        }

        let result = self.apply_relocations(address_space, resolver);
        self.state = if result.is_ok() {
            ObjectState::ReadyToSeal
        } else {
            ObjectState::Failed
        };
        result
    }

    pub(crate) fn seal<A: AddressSpace>(
        &mut self,
        address_space: &mut A,
    ) -> Result<(), Error> {
        if self.state != ObjectState::ReadyToSeal {
            return Err(Error::Inval);
        }
        let page_count = self.mapping_length / PAGE_SIZE;
        let mut page_permissions = Vec::new();
        page_permissions
            .try_reserve_exact(page_count)
            .map_err(|_| Error::NoMem)?;
        page_permissions.resize(page_count, 0u8);

        for segment in &self.load_ranges {
            let first_page = segment.start & !((PAGE_SIZE as u64) - 1);
            let end_page = align_up(
                usize::try_from(segment.end).map_err(|_| Error::Overflow)?,
                PAGE_SIZE,
            )
            .ok_or(Error::Overflow)? as u64;
            let start_index = self.page_index(first_page)?;
            let end_index = self.page_index(end_page)?;
            let mut permissions = 0u8;
            if segment.permissions & PF_READ != 0 {
                permissions |= Permissions::READ.bits();
            }
            if segment.permissions & PF_WRITE != 0 {
                permissions |= Permissions::WRITE.bits();
            }
            if segment.permissions & PF_EXECUTE != 0 {
                permissions |= Permissions::EXECUTE.bits();
            }
            for page in &mut page_permissions[start_index..end_index] {
                *page |= permissions;
            }
        }

        for segment in &self.relro_segments {
            let segment_end = segment
                .virtual_address
                .checked_add(segment.memory_size)
                .ok_or(Error::Overflow)?;
            let first_page = segment.virtual_address & !((PAGE_SIZE as u64) - 1);
            let end_page = segment_end & !((PAGE_SIZE as u64) - 1);
            if end_page <= first_page {
                continue;
            }
            let start_index = self.page_index(first_page)?;
            let end_index = self.page_index(end_page)?;
            for page in &mut page_permissions[start_index..end_index] {
                *page &= !Permissions::WRITE.bits();
            }
        }

        let mut page_index = 0usize;
        while page_index < page_count {
            let permissions = page_permissions[page_index];
            if permissions & (Permissions::WRITE.bits() | Permissions::EXECUTE.bits())
                == (Permissions::WRITE.bits() | Permissions::EXECUTE.bits())
            {
                self.state = ObjectState::Failed;
                return Err(Error::OpNotSupp);
            }
            let first_page = page_index;
            page_index += 1;
            while page_index < page_count && page_permissions[page_index] == permissions {
                page_index += 1;
            }
            let address = self
                .mapping_base
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
                self.state = ObjectState::Failed;
                return Err(error);
            }
        }
        self.state = ObjectState::Sealed;
        Ok(())
    }

    pub(crate) fn release<A: AddressSpace>(self, address_space: &mut A) {
        address_space.release(self.mapping_base, self.mapping_length);
    }

    fn page_index(&self, virtual_page: u64) -> Result<usize, Error> {
        let runtime_page = self
            .load_bias
            .checked_add(usize::try_from(virtual_page).map_err(|_| Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        let offset = runtime_page
            .checked_sub(self.mapping_base)
            .ok_or(Error::NoExec)?;
        if offset > self.mapping_length || offset % PAGE_SIZE != 0 {
            return Err(Error::NoExec);
        }
        Ok(offset / PAGE_SIZE)
    }

    fn apply_relocations<A: AddressSpace, R: SymbolResolver>(
        &self,
        address_space: &mut A,
        resolver: &mut R,
    ) -> Result<(), Error> {
        for relocation in &self.relocations {
            if relocation.kind == R_NONE {
                continue;
            }
            let width = relocation_width(relocation.kind).ok_or(Error::OpNotSupp)?;
            let target = self
                .load_bias
                .checked_add(usize::try_from(relocation.offset).map_err(|_| Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            if !self.writable_target(relocation.offset, width)? {
                return Err(Error::Libbad);
            }
            let addend = match relocation.addend {
                Some(addend) => addend,
                None => read_implicit_addend(address_space, target, relocation.kind, width)?,
            };
            let relative = relocation.kind == relative_relocation_type();
            if relative && relocation.symbol_index != 0 {
                return Err(Error::Libbad);
            }
            let symbol = if relative {
                0
            } else {
                self.resolve_relocation_symbol(relocation.symbol_index, resolver)?
            };
            let Some(value) = calculate_relocation(
                relocation.kind,
                symbol,
                addend,
                self.load_bias,
                target,
            )? else {
                continue;
            };
            let bytes = value.to_le_bytes();
            address_space.write(target, &bytes[..width])?;
        }
        Ok(())
    }

    fn resolve_relocation_symbol<R: SymbolResolver>(
        &self,
        symbol_index: u32,
        resolver: &mut R,
    ) -> Result<usize, Error> {
        if symbol_index == 0 {
            return Ok(0);
        }
        let symbol = self
            .symbols
            .get(symbol_index as usize)
            .ok_or(Error::NoExec)?;
        if matches!(symbol.symbol_type, STT_TLS | STT_GNU_IFUNC) {
            return Err(Error::OpNotSupp);
        }
        if symbol.section_index != SHN_UNDEFINED && symbol.visibility == STV_PROTECTED {
            return symbol.address.ok_or(Error::NoExec);
        }
        if symbol.section_index == SHN_UNDEFINED {
            if symbol.binding == 0 || symbol.visibility != STV_DEFAULT || symbol.name.is_empty() {
                return Err(Error::NoExec);
            }
            if let Some(resolved) = resolver.resolve(&symbol.name) {
                return Ok(resolved.address);
            }
            return if symbol.binding == STB_WEAK {
                Ok(0)
            } else {
                Err(Error::Libbad)
            };
        }
        if matches!(symbol.binding, STB_GLOBAL | STB_WEAK)
            && matches!(symbol.visibility, STV_DEFAULT | STV_PROTECTED)
        {
            if let Some(resolved) = resolver.resolve(&symbol.name) {
                return Ok(resolved.address);
            }
        }
        symbol.address.ok_or(Error::NoExec)
    }

    fn writable_target(&self, virtual_address: u64, width: usize) -> Result<bool, Error> {
        let end = virtual_address
            .checked_add(width as u64)
            .ok_or(Error::Overflow)?;
        let index = self
            .load_ranges
            .partition_point(|segment| segment.end <= virtual_address);
        Ok(self.load_ranges.get(index).is_some_and(|segment| {
            segment.permissions & PF_WRITE != 0
                && virtual_address >= segment.start
                && end <= segment.end
        }))
    }

    fn load_reserved<F: FileSource, A: AddressSpace>(
        parsed: &ParsedElf<'_, F>,
        address_space: &mut A,
        mapping_base: usize,
        load_bias: usize,
    ) -> Result<Self, Error> {
        address_space.zero(mapping_base, parsed.mapping_length)?;
        let mut relro_segments = Vec::new();
        relro_segments
            .try_reserve_exact(parsed.program_headers.len())
            .map_err(|_| Error::NoMem)?;

        for header in &parsed.program_headers {
            match header.kind {
                PT_LOAD if header.memory_size != 0 => {
                    let virtual_address = usize::try_from(header.virtual_address)
                        .map_err(|_| Error::Overflow)?;
                    let target = load_bias
                        .checked_add(virtual_address)
                        .ok_or(Error::Overflow)?;
                    copy_file_range(
                        parsed.source,
                        address_space,
                        header.offset,
                        target,
                        header.file_size,
                    )?;
                }
                PT_GNU_RELRO if header.memory_size != 0 => {
                    relro_segments.push(*header);
                }
                _ => {}
            }
        }

        let (needed, symbols, relocations) = parsed.load_dynamic(load_bias)?;
        let mut load_ranges = Vec::new();
        load_ranges
            .try_reserve_exact(parsed.load_ranges.len())
            .map_err(|_| Error::NoMem)?;
        load_ranges.extend_from_slice(&parsed.load_ranges);

        let entry = if parsed.kind != ObjectKind::Executable || parsed.entry == 0 {
            None
        } else {
            Some(
                load_bias
                    .checked_add(usize::try_from(parsed.entry).map_err(|_| Error::Overflow)?)
                    .ok_or(Error::Overflow)?,
            )
        };

        Ok(Self {
            kind: parsed.kind,
            mapping_base,
            mapping_length: parsed.mapping_length,
            load_bias,
            entry,
            load_ranges,
            relro_segments,
            needed,
            symbols,
            state: if relocations.is_empty() {
                ObjectState::ReadyToSeal
            } else {
                ObjectState::RelocationPending
            },
            relocations,
        })
    }
}

impl<'a, F: FileSource> ParsedElf<'a, F> {
    fn parse(source: &'a F, requested_kind: ObjectKind) -> Result<Self, Error> {
        if source.len() < ELF_HEADER_SIZE as u64 {
            return Err(Error::NoExec);
        }
        let mut header = [0u8; ELF_HEADER_SIZE];
        read_exact(source, 0, &mut header)?;
        if header[..4] != *b"\x7fELF"
            || header[4] != 2
            || header[5] != 1
            || header[6] != 1
            || endian::read_u32(&header, 20).ok_or(Error::NoExec)? != 1
            || endian::read_u16(&header, 52).ok_or(Error::NoExec)? as usize != ELF_HEADER_SIZE
            || endian::read_u16(&header, 54).ok_or(Error::NoExec)? as usize != PROGRAM_HEADER_SIZE
        {
            return Err(Error::NoExec);
        }

        let elf_type = endian::read_u16(&header, 16).ok_or(Error::NoExec)?;
        let kind = match (elf_type, requested_kind) {
            (ET_EXEC, ObjectKind::Executable) => ObjectKind::Executable,
            (ET_EXEC, ObjectKind::SharedObject) => return Err(Error::NoExec),
            (ET_DYN, requested_kind) => requested_kind,
            _ => return Err(Error::OpNotSupp),
        };
        if endian::read_u16(&header, 18).ok_or(Error::NoExec)? != native_machine() {
            return Err(Error::OpNotSupp);
        }

        let entry = endian::read_u64(&header, 24).ok_or(Error::NoExec)?;
        let program_offset = endian::read_u64(&header, 32).ok_or(Error::NoExec)?;
        let program_count = endian::read_u16(&header, 56).ok_or(Error::NoExec)? as usize;
        if program_count == 0 || program_count > MAX_PROGRAM_HEADERS {
            return Err(Error::NoExec);
        }
        let program_bytes = program_count
            .checked_mul(PROGRAM_HEADER_SIZE)
            .ok_or(Error::Overflow)?;
        let program_end = program_offset
            .checked_add(program_bytes as u64)
            .ok_or(Error::Overflow)?;
        if program_end > source.len() {
            return Err(Error::NoExec);
        }

        let mut program_headers = Vec::new();
        program_headers
            .try_reserve_exact(program_count)
            .map_err(|_| Error::NoMem)?;
        let mut minimum = usize::MAX;
        let mut maximum = 0usize;
        let mut alignment = PAGE_SIZE;
        let mut has_executable_entry = false;
        let mut load_ranges = Vec::new();
        let mut file_ranges = Vec::new();
        load_ranges
            .try_reserve_exact(program_count)
            .map_err(|_| Error::NoMem)?;
        file_ranges
            .try_reserve_exact(program_count)
            .map_err(|_| Error::NoMem)?;
        for index in 0..program_count {
            let offset = program_offset
                .checked_add((index * PROGRAM_HEADER_SIZE) as u64)
                .ok_or(Error::Overflow)?;
            let mut bytes = [0u8; PROGRAM_HEADER_SIZE];
            read_exact(source, offset, &mut bytes)?;
            let header = ProgramHeader {
                kind: endian::read_u32(&bytes, 0).ok_or(Error::NoExec)?,
                flags: endian::read_u32(&bytes, 4).ok_or(Error::NoExec)?,
                offset: endian::read_u64(&bytes, 8).ok_or(Error::NoExec)?,
                virtual_address: endian::read_u64(&bytes, 16).ok_or(Error::NoExec)?,
                file_size: endian::read_u64(&bytes, 32).ok_or(Error::NoExec)?,
                memory_size: endian::read_u64(&bytes, 40).ok_or(Error::NoExec)?,
                alignment: endian::read_u64(&bytes, 48).ok_or(Error::NoExec)?,
            };
            let file_end = header
                .offset
                .checked_add(header.file_size)
                .ok_or(Error::Overflow)?;
            if file_end > source.len() {
                return Err(Error::NoExec);
            }
            if header.kind == PT_LOAD {
                if header.file_size > header.memory_size {
                    return Err(Error::NoExec);
                }
                if header.flags & !(PF_READ | PF_WRITE | PF_EXECUTE) != 0 {
                    return Err(Error::OpNotSupp);
                }
            }
            if header.kind == PT_LOAD && header.memory_size != 0 {
                let virtual_end = header
                    .virtual_address
                    .checked_add(header.memory_size)
                    .ok_or(Error::Overflow)?;
                load_ranges.push(LoadRange {
                    start: header.virtual_address,
                    end: virtual_end,
                    permissions: header.flags & (PF_READ | PF_WRITE | PF_EXECUTE),
                });
                if header.file_size != 0 {
                    let file_virtual_end = header
                        .virtual_address
                        .checked_add(header.file_size)
                        .ok_or(Error::Overflow)?;
                    file_ranges.push(FileRange {
                        start: header.virtual_address,
                        end: file_virtual_end,
                        file_offset: header.offset,
                    });
                }
                let segment_alignment = usize::try_from(header.alignment)
                    .map_err(|_| Error::Overflow)?;
                if segment_alignment > 1 {
                    if !segment_alignment.is_power_of_two()
                        || segment_alignment > MAX_IMAGE_ALIGNMENT
                        || header.offset % header.alignment
                            != header.virtual_address % header.alignment
                    {
                        return Err(Error::NoExec);
                    }
                    alignment = alignment.max(segment_alignment);
                }
                let virtual_start = usize::try_from(header.virtual_address)
                    .map_err(|_| Error::Overflow)?;
                let virtual_end = usize::try_from(virtual_end).map_err(|_| Error::Overflow)?;
                minimum = minimum.min(virtual_start & !(PAGE_SIZE - 1));
                maximum = maximum.max(align_up(virtual_end, PAGE_SIZE).ok_or(Error::Overflow)?);
                if header.flags & PF_EXECUTE != 0
                    && entry >= header.virtual_address
                    && entry < header.virtual_address + header.memory_size
                {
                    has_executable_entry = true;
                }
            }
            if matches!(header.kind, PT_INTERP | PT_TLS) {
                return Err(Error::OpNotSupp);
            }
            program_headers.push(header);
        }

        if minimum == usize::MAX || maximum <= minimum {
            return Err(Error::NoExec);
        }
        let mapping_length = maximum - minimum;
        if mapping_length > MAX_IMAGE_SIZE {
            return Err(Error::Fbig);
        }
        load_ranges.sort_unstable_by_key(|range| range.start);
        if load_ranges
            .windows(2)
            .any(|ranges| ranges[1].start < ranges[0].end)
        {
            return Err(Error::NoExec);
        }
        file_ranges.sort_unstable_by_key(|range| range.start);
        if file_ranges
            .windows(2)
            .any(|ranges| ranges[1].start < ranges[0].end)
        {
            return Err(Error::NoExec);
        }
        if kind == ObjectKind::Executable && (entry == 0 || !has_executable_entry) {
            return Err(Error::NoExec);
        }
        for relro in program_headers
            .iter()
            .filter(|header| header.kind == PT_GNU_RELRO && header.memory_size != 0)
        {
            let relro_end = relro
                .virtual_address
                .checked_add(relro.memory_size)
                .ok_or(Error::Overflow)?;
            let inside_writable_load = program_headers.iter().any(|load| {
                if load.kind != PT_LOAD || load.flags & PF_WRITE == 0 {
                    return false;
                }
                let Some(load_end) = load.virtual_address.checked_add(load.memory_size) else {
                    return false;
                };
                relro.virtual_address >= load.virtual_address && relro_end <= load_end
            });
            if !inside_writable_load {
                return Err(Error::NoExec);
            }
        }
        validate_page_permissions(&load_ranges)?;

        Ok(Self {
            source,
            kind,
            fixed_address: elf_type == ET_EXEC,
            entry,
            program_headers,
            load_ranges,
            file_ranges,
            minimum_page: minimum,
            mapping_length,
            alignment,
        })
    }

    fn load_dynamic(
        &self,
        load_bias: usize,
    ) -> Result<(Vec<Vec<u8>>, Vec<DynamicSymbol>, Vec<Relocation>), Error> {
        let Some(dynamic_header) = self
            .program_headers
            .iter()
            .find(|header| header.kind == PT_DYNAMIC)
        else {
            return Ok((Vec::new(), Vec::new(), Vec::new()));
        };
        if self
            .program_headers
            .iter()
            .filter(|header| header.kind == PT_DYNAMIC)
            .count()
            != 1
            || dynamic_header.file_size == 0
            || dynamic_header.file_size > dynamic_header.memory_size
            || dynamic_header.file_size % DYNAMIC_ENTRY_SIZE as u64 != 0
        {
            return Err(Error::NoExec);
        }
        if self.virtual_file_offset(
            dynamic_header.virtual_address,
            usize::try_from(dynamic_header.file_size).map_err(|_| Error::Overflow)?,
        )? != dynamic_header.offset
        {
            return Err(Error::NoExec);
        }
        let entry_count = usize::try_from(dynamic_header.file_size / DYNAMIC_ENTRY_SIZE as u64)
            .map_err(|_| Error::Overflow)?;
        if entry_count > MAX_DYNAMIC_ENTRIES {
            return Err(Error::Fbig);
        }

        let mut dynamic = DynamicInfo {
            tags: Vec::new(),
            needed_offsets: Vec::new(),
        };
        dynamic
            .tags
            .try_reserve_exact(entry_count)
            .map_err(|_| Error::NoMem)?;
        let mut terminated = false;
        for index in 0..entry_count {
            let entry_offset = dynamic_header
                .offset
                .checked_add((index * DYNAMIC_ENTRY_SIZE) as u64)
                .ok_or(Error::Overflow)?;
            let mut entry = [0u8; DYNAMIC_ENTRY_SIZE];
            read_exact(self.source, entry_offset, &mut entry)?;
            let tag = endian::read_u64(&entry, 0).ok_or(Error::NoExec)? as i64;
            let value = endian::read_u64(&entry, 8).ok_or(Error::NoExec)?;
            if tag == DT_NULL {
                terminated = true;
                break;
            }
            if tag == DT_NEEDED {
                dynamic
                    .needed_offsets
                    .try_reserve(1)
                    .map_err(|_| Error::NoMem)?;
                dynamic.needed_offsets.push(value);
            }
            dynamic.tags.push((tag, value));
        }
        if !terminated {
            return Err(Error::NoExec);
        }
        if dynamic.value(DT_TEXTREL)?.is_some()
            || dynamic.value(DT_FLAGS)?.is_some_and(|flags| flags & DF_TEXTREL != 0)
        {
            return Err(Error::OpNotSupp);
        }
        if dynamic.value(DT_SYMBOLIC)?.is_some()
            || dynamic
                .value(DT_FLAGS)?
                .is_some_and(|flags| flags & DF_SYMBOLIC != 0)
        {
            return Err(Error::OpNotSupp);
        }
        if [DT_VERSYM, DT_VERDEF, DT_VERDEFNUM, DT_VERNEED, DT_VERNEEDNUM]
            .iter()
            .any(|tag| dynamic.tags.iter().any(|(present, _)| present == tag))
        {
            return Err(Error::OpNotSupp);
        }

        let string_address = dynamic.value(DT_STRTAB)?;
        let string_size = dynamic.value(DT_STRSZ)?;
        let needs_string_table = !dynamic.needed_offsets.is_empty()
            || dynamic.value(DT_SYMTAB)?.is_some();
        match (string_address, string_size) {
            (Some(address), Some(size)) => {
                if size == 0 {
                    return Err(Error::NoExec);
                }
                let mut first_byte = [1u8];
                self.read_virtual(address, &mut first_byte)?;
                if first_byte[0] != 0 {
                    return Err(Error::NoExec);
                }
            }
            (None, None) if !needs_string_table => {}
            _ => return Err(Error::NoExec),
        }
        let mut needed = Vec::new();
        needed
            .try_reserve_exact(dynamic.needed_offsets.len())
            .map_err(|_| Error::NoMem)?;
        if let (Some(string_address), Some(string_size)) = (string_address, string_size) {
            if string_size > MAX_STRING_TABLE_SIZE {
                return Err(Error::Fbig);
            }
            let mut total_name_bytes = 0usize;
            for name_offset in &dynamic.needed_offsets {
                let name = self.read_string(string_address, string_size, *name_offset)?;
                if name.is_empty() {
                    return Err(Error::NoExec);
                }
                total_name_bytes = total_name_bytes
                    .checked_add(name.len())
                    .ok_or(Error::Overflow)?;
                if total_name_bytes > MAX_STRING_TABLE_SIZE as usize {
                    return Err(Error::Fbig);
                }
                needed.push(name);
            }
        }

        let symbols = self.load_symbols(&dynamic, load_bias)?;
        let relocations = self.load_relocations(&dynamic, symbols.len())?;
        Ok((needed, symbols, relocations))
    }

    fn load_symbols(
        &self,
        dynamic: &DynamicInfo,
        load_bias: usize,
    ) -> Result<Vec<DynamicSymbol>, Error> {
        let Some(symbol_address) = dynamic.value(DT_SYMTAB)? else {
            return Ok(Vec::new());
        };
        let string_address = dynamic.value(DT_STRTAB)?.ok_or(Error::NoExec)?;
        let string_size = dynamic.value(DT_STRSZ)?.ok_or(Error::NoExec)?;
        if string_size > MAX_STRING_TABLE_SIZE
            || dynamic.value(DT_SYMENT)? != Some(SYMBOL_ENTRY_SIZE as u64)
        {
            return Err(Error::OpNotSupp);
        }
        let symbol_count = self.symbol_count(dynamic)?;
        if symbol_count == 0 {
            return Err(Error::NoExec);
        }
        if symbol_count > MAX_DYNAMIC_SYMBOLS {
            return Err(Error::Fbig);
        }
        let mut symbols = Vec::new();
        symbols
            .try_reserve_exact(symbol_count)
            .map_err(|_| Error::NoMem)?;
        let mut total_name_bytes = 0usize;
        for index in 0..symbol_count {
            let symbol_offset = symbol_address
                .checked_add(
                    (index as u64)
                        .checked_mul(SYMBOL_ENTRY_SIZE as u64)
                        .ok_or(Error::Overflow)?,
                )
                .ok_or(Error::Overflow)?;
            let mut bytes = [0u8; SYMBOL_ENTRY_SIZE];
            self.read_virtual(symbol_offset, &mut bytes)?;
            let name_offset = endian::read_u32(&bytes, 0).ok_or(Error::NoExec)? as u64;
            if name_offset >= string_size {
                return Err(Error::NoExec);
            }
            let info = bytes[4];
            let other = bytes[5];
            let section_index = endian::read_u16(&bytes, 6).ok_or(Error::NoExec)?;
            let value = endian::read_u64(&bytes, 8).ok_or(Error::NoExec)?;
            let size = endian::read_u64(&bytes, 16).ok_or(Error::NoExec)?;
            let symbol_type = info & 0x0f;
            let binding = info >> 4;
            if index == 0
                && (name_offset != 0
                    || info != 0
                    || other != 0
                    || section_index != SHN_UNDEFINED
                    || value != 0
                    || size != 0)
            {
                return Err(Error::NoExec);
            }
            if !matches!(binding, 0 | STB_GLOBAL | STB_WEAK) || other & !0x03 != 0 {
                return Err(Error::OpNotSupp);
            }
            if symbol_type == STT_TLS || symbol_type == STT_GNU_IFUNC {
                return Err(Error::OpNotSupp);
            }
            if section_index >= SHN_LORESERVE && section_index != SHN_ABSOLUTE {
                return Err(Error::OpNotSupp);
            }
            let name = self.read_string(string_address, string_size, name_offset)?;
            total_name_bytes = total_name_bytes
                .checked_add(name.len())
                .ok_or(Error::Overflow)?;
            if total_name_bytes > MAX_STRING_TABLE_SIZE as usize {
                return Err(Error::Fbig);
            }
            let address = if section_index == SHN_UNDEFINED {
                None
            } else if section_index == SHN_ABSOLUTE {
                Some(usize::try_from(value).map_err(|_| Error::Overflow)?)
            } else {
                if !self.contains_virtual_range(value, size) {
                    return Err(Error::NoExec);
                }
                Some(
                    load_bias
                        .checked_add(usize::try_from(value).map_err(|_| Error::Overflow)?)
                        .ok_or(Error::Overflow)?,
                )
            };
            symbols.push(DynamicSymbol {
                name,
                address,
                size,
                section_index,
                binding,
                symbol_type,
                visibility: other & 0x03,
            });
        }
        Ok(symbols)
    }

    fn contains_virtual_range(&self, address: u64, length: u64) -> bool {
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.program_headers.iter().any(|segment| {
            if segment.kind != PT_LOAD {
                return false;
            }
            let Some(segment_end) = segment.virtual_address.checked_add(segment.memory_size) else {
                return false;
            };
            address >= segment.virtual_address
                && if length == 0 {
                    address <= segment_end
                } else {
                    end <= segment_end
                }
        })
    }

    fn load_relocations(
        &self,
        dynamic: &DynamicInfo,
        symbol_count: usize,
    ) -> Result<Vec<Relocation>, Error> {
        let mut relocations = Vec::new();
        let mut tables: Vec<(u64, u64, bool)> = Vec::new();
        tables
            .try_reserve_exact(2)
            .map_err(|_| Error::NoMem)?;
        if let Some(address) = dynamic.value(DT_RELA)? {
            let size = dynamic.value(DT_RELASZ)?.ok_or(Error::NoExec)?;
            if dynamic.value(DT_RELAENT)? != Some(24) {
                return Err(Error::OpNotSupp);
            }
            self.append_relocations(address, size, true, symbol_count, &mut relocations)?;
            tables.push((address, size, true));
        } else if dynamic.value(DT_RELASZ)?.is_some() {
            return Err(Error::NoExec);
        }
        if let Some(address) = dynamic.value(DT_REL)? {
            let size = dynamic.value(DT_RELSZ)?.ok_or(Error::NoExec)?;
            if dynamic.value(DT_RELENT)? != Some(16) {
                return Err(Error::OpNotSupp);
            }
            self.append_relocations(address, size, false, symbol_count, &mut relocations)?;
            tables.push((address, size, false));
        } else if dynamic.value(DT_RELSZ)?.is_some() {
            return Err(Error::NoExec);
        }

        if let Some(address) = dynamic.value(DT_JMPREL)? {
            let size = dynamic.value(DT_PLTRELSZ)?.ok_or(Error::NoExec)?;
            let rela = match dynamic.value(DT_PLTREL)? {
                Some(7) => true,
                Some(17) => false,
                _ => return Err(Error::OpNotSupp),
            };
            if !tables.contains(&(address, size, rela)) {
                self.append_relocations(address, size, rela, symbol_count, &mut relocations)?;
            }
        } else if dynamic.value(DT_PLTRELSZ)?.is_some() {
            return Err(Error::NoExec);
        }

        match (
            dynamic.value(DT_RELR)?,
            dynamic.value(DT_RELRSZ)?,
            dynamic.value(DT_RELRENT)?,
        ) {
            (None, None, None) => {}
            (Some(address), Some(size), Some(8)) => {
                self.append_relr(address, size, &mut relocations)?;
            }
            _ => return Err(Error::OpNotSupp),
        }
        relocations.sort_unstable_by_key(|relocation| relocation.offset);
        let mut previous_end = 0u64;
        let mut has_previous = false;
        for relocation in &relocations {
            if relocation.kind == R_NONE {
                continue;
            }
            let width = relocation_width(relocation.kind).ok_or(Error::OpNotSupp)? as u64;
            let end = relocation
                .offset
                .checked_add(width)
                .ok_or(Error::Overflow)?;
            if has_previous && relocation.offset < previous_end {
                return Err(Error::Libbad);
            }
            previous_end = end;
            has_previous = true;
        }
        Ok(relocations)
    }

    fn append_relocations(
        &self,
        address: u64,
        size: u64,
        rela: bool,
        symbol_count: usize,
        output: &mut Vec<Relocation>,
    ) -> Result<(), Error> {
        let entry_size = if rela { 24usize } else { 16usize };
        if size % entry_size as u64 != 0 {
            return Err(Error::NoExec);
        }
        let count = usize::try_from(size / entry_size as u64).map_err(|_| Error::Overflow)?;
        if output.len().checked_add(count).ok_or(Error::Overflow)? > MAX_DYNAMIC_RELOCATIONS {
            return Err(Error::Fbig);
        }
        output
            .try_reserve(count)
            .map_err(|_| Error::NoMem)?;
        for index in 0..count {
            let entry_address = address
                .checked_add((index as u64).checked_mul(entry_size as u64).ok_or(Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            let mut bytes = [0u8; 24];
            self.read_virtual(entry_address, &mut bytes[..entry_size])?;
            let offset = endian::read_u64(&bytes, 0).ok_or(Error::NoExec)?;
            let info = endian::read_u64(&bytes, 8).ok_or(Error::NoExec)?;
            let symbol_index = u32::try_from(info >> 32).map_err(|_| Error::Overflow)?;
            let kind = info as u32;
            let width = relocation_width(kind).ok_or(Error::OpNotSupp)?;
            let addend = if rela {
                Some(endian::read_u64(&bytes, 16).ok_or(Error::NoExec)? as i64)
            } else {
                None
            };
            if kind != R_NONE {
                if symbol_index as usize >= symbol_count && symbol_index != 0 {
                    return Err(Error::NoExec);
                }
                if !self.has_writable_segment(offset, width)? {
                    return Err(Error::Libbad);
                }
            }
            output.push(Relocation {
                offset,
                symbol_index,
                kind,
                addend,
            });
        }
        Ok(())
    }

    fn append_relr(
        &self,
        address: u64,
        size: u64,
        output: &mut Vec<Relocation>,
    ) -> Result<(), Error> {
        if size % 8 != 0 {
            return Err(Error::NoExec);
        }
        let count = usize::try_from(size / 8).map_err(|_| Error::Overflow)?;
        if count > MAX_DYNAMIC_RELOCATIONS {
            return Err(Error::Fbig);
        }
        let mut where_address = None;
        for index in 0..count {
            let entry_address = address
                .checked_add((index as u64).checked_mul(8).ok_or(Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            let mut bytes = [0u8; 8];
            self.read_virtual(entry_address, &mut bytes)?;
            let entry = endian::read_u64(&bytes, 0).ok_or(Error::NoExec)?;
            if entry & 1 == 0 {
                self.push_relr(entry, output)?;
                where_address = Some(entry.checked_add(8).ok_or(Error::Overflow)?);
                continue;
            }
            let mut base = where_address.ok_or(Error::NoExec)?;
            for bit in 1..64 {
                if entry & (1u64 << bit) != 0 {
                    let offset = base
                        .checked_add(((bit - 1) * 8) as u64)
                        .ok_or(Error::Overflow)?;
                    self.push_relr(offset, output)?;
                }
            }
            base = base.checked_add(63 * 8).ok_or(Error::Overflow)?;
            where_address = Some(base);
        }
        Ok(())
    }

    fn push_relr(&self, offset: u64, output: &mut Vec<Relocation>) -> Result<(), Error> {
        if !self.has_writable_segment(offset, 8)? {
            return Err(Error::Libbad);
        }
        if output.len() >= MAX_DYNAMIC_RELOCATIONS {
            return Err(Error::Fbig);
        }
        output.try_reserve(1).map_err(|_| Error::NoMem)?;
        output.push(Relocation {
            offset,
            symbol_index: 0,
            kind: relative_relocation_type(),
            addend: None,
        });
        Ok(())
    }

    fn has_writable_segment(&self, address: u64, width: usize) -> Result<bool, Error> {
        let end = address
            .checked_add(width as u64)
            .ok_or(Error::Overflow)?;
        let index = self.load_ranges.partition_point(|range| range.end <= address);
        Ok(self.load_ranges.get(index).is_some_and(|range| {
            range.permissions & PF_WRITE != 0 && address >= range.start && end <= range.end
        }))
    }

    fn symbol_count(&self, dynamic: &DynamicInfo) -> Result<usize, Error> {
        if let Some(hash_address) = dynamic.value(DT_HASH)? {
            let mut header = [0u8; 8];
            self.read_virtual(hash_address, &mut header)?;
            return usize::try_from(endian::read_u32(&header, 4).ok_or(Error::NoExec)?).map_err(|_| Error::Overflow);
        }
        let Some(hash_address) = dynamic.value(DT_GNU_HASH)? else {
            return Err(Error::OpNotSupp);
        };
        let mut header = [0u8; 16];
        self.read_virtual(hash_address, &mut header)?;
        let bucket_count = endian::read_u32(&header, 0).ok_or(Error::NoExec)? as usize;
        let symbol_offset = endian::read_u32(&header, 4).ok_or(Error::NoExec)? as usize;
        let bloom_count = endian::read_u32(&header, 8).ok_or(Error::NoExec)? as usize;
        if bucket_count > MAX_DYNAMIC_SYMBOLS || bloom_count == 0 || bloom_count > MAX_DYNAMIC_SYMBOLS
        {
            return Err(Error::NoExec);
        }
        let bucket_address = hash_address
            .checked_add(16)
            .and_then(|value| value.checked_add((bloom_count as u64).checked_mul(8)?))
            .ok_or(Error::Overflow)?;
        let chain_address = bucket_address
            .checked_add((bucket_count as u64).checked_mul(4).ok_or(Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        let mut symbol_count = symbol_offset;
        let mut total_chain_steps = 0usize;
        let maximum_chain_count = usize::try_from(self.file_bytes_remaining(chain_address)? / 4)
            .map_err(|_| Error::Overflow)?
            .min(MAX_DYNAMIC_SYMBOLS.saturating_sub(symbol_offset));
        for bucket_index in 0..bucket_count {
            let bucket_offset = bucket_address
                .checked_add((bucket_index as u64).checked_mul(4).ok_or(Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            let mut bucket_bytes = [0u8; 4];
            self.read_virtual(bucket_offset, &mut bucket_bytes)?;
            let mut symbol_index = endian::read_u32(&bucket_bytes, 0).ok_or(Error::NoExec)? as usize;
            if symbol_index < symbol_offset {
                continue;
            }
            loop {
                let chain_index = symbol_index
                    .checked_sub(symbol_offset)
                    .ok_or(Error::Overflow)?;
                if chain_index >= maximum_chain_count {
                    return Err(Error::NoExec);
                }
                let chain_offset = chain_address
                    .checked_add((chain_index as u64).checked_mul(4).ok_or(Error::Overflow)?)
                    .ok_or(Error::Overflow)?;
                let mut chain_bytes = [0u8; 4];
                self.read_virtual(chain_offset, &mut chain_bytes)?;
                let chain = endian::read_u32(&chain_bytes, 0).ok_or(Error::NoExec)?;
                symbol_count = symbol_count.max(symbol_index.checked_add(1).ok_or(Error::Overflow)?);
                total_chain_steps = total_chain_steps
                    .checked_add(1)
                    .ok_or(Error::Overflow)?;
                if total_chain_steps > MAX_DYNAMIC_SYMBOLS {
                    return Err(Error::Fbig);
                }
                if chain & 1 != 0 {
                    break;
                }
                symbol_index = symbol_index.checked_add(1).ok_or(Error::Overflow)?;
            }
        }
        Ok(symbol_count)
    }

    fn read_string(
        &self,
        table_address: u64,
        table_size: u64,
        name_offset: u64,
    ) -> Result<Vec<u8>, Error> {
        if name_offset >= table_size {
            return Err(Error::NoExec);
        }
        let mut output = Vec::new();
        let mut offset = name_offset;
        let mut buffer = [0u8; 128];
        while offset < table_size {
            let address = table_address.checked_add(offset).ok_or(Error::Overflow)?;
            let available = (table_size - offset)
                .min(buffer.len() as u64)
                .min(self.file_bytes_remaining(address)?) as usize;
            if available == 0 {
                return Err(Error::NoExec);
            }
            self.read_virtual(address, &mut buffer[..available])?;
            let end = buffer[..available]
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(available);
            if output.len().checked_add(end).ok_or(Error::Overflow)? > MAX_SYMBOL_NAME_SIZE {
                return Err(Error::Fbig);
            }
            output
                .try_reserve(end)
                .map_err(|_| Error::NoMem)?;
            output.extend_from_slice(&buffer[..end]);
            if end != available {
                return Ok(output);
            }
            offset = offset
                .checked_add(available as u64)
                .ok_or(Error::Overflow)?;
        }
        Err(Error::NoExec)
    }

    fn read_virtual(&self, address: u64, buffer: &mut [u8]) -> Result<(), Error> {
        let file_offset = self.virtual_file_offset(address, buffer.len())?;
        read_exact(self.source, file_offset, buffer)
    }

    fn virtual_file_offset(&self, address: u64, length: usize) -> Result<u64, Error> {
        let length = length as u64;
        let end = address.checked_add(length).ok_or(Error::Overflow)?;
        let index = self
            .file_ranges
            .partition_point(|range| range.end <= address);
        let range = self.file_ranges.get(index).ok_or(Error::NoExec)?;
        if address < range.start || end > range.end {
            return Err(Error::NoExec);
        }
        range
            .file_offset
            .checked_add(address - range.start)
            .ok_or(Error::Overflow)
    }

    fn file_bytes_remaining(&self, address: u64) -> Result<u64, Error> {
        let index = self
            .file_ranges
            .partition_point(|range| range.end <= address);
        let range = self.file_ranges.get(index).ok_or(Error::NoExec)?;
        if address < range.start {
            return Err(Error::NoExec);
        }
        Ok(range.end - address)
    }
}

impl DynamicInfo {
    fn value(&self, requested_tag: i64) -> Result<Option<u64>, Error> {
        let mut found = None;
        for (tag, value) in &self.tags {
            if *tag == requested_tag {
                if found.is_some_and(|previous| previous != *value) {
                    return Err(Error::NoExec);
                }
                found = Some(*value);
            }
        }
        Ok(found)
    }
}

fn read_implicit_addend<A: AddressSpace>(
    address_space: &A,
    address: usize,
    kind: u32,
    width: usize,
) -> Result<i64, Error> {
    let mut bytes = [0u8; 8];
    address_space.read(address, &mut bytes[..width])?;
    match (kind, width) {
        #[cfg(target_arch = "x86_64")]
        (10, 4) => Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64),
        #[cfg(target_arch = "x86_64")]
        (2 | 4 | 11, 4) => {
            Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64)
        }
        #[cfg(target_arch = "aarch64")]
        (258, 4) => Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64),
        #[cfg(target_arch = "aarch64")]
        (261, 4) => Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i64),
        (_, 4) => Err(Error::OpNotSupp),
        (_, 8) => Ok(i64::from_le_bytes(bytes)),
        _ => Err(Error::OpNotSupp),
    }
}

#[cfg(target_arch = "x86_64")]
const fn relative_relocation_type() -> u32 {
    8
}

#[cfg(target_arch = "aarch64")]
const fn relative_relocation_type() -> u32 {
    1027
}

#[cfg(target_arch = "x86_64")]
fn relocation_width(kind: u32) -> Option<usize> {
    match kind {
        R_NONE => Some(0),
        1 | 6 | 7 | 8 => Some(8),
        2 | 4 | 10 | 11 => Some(4),
        _ => None,
    }
}

#[cfg(target_arch = "aarch64")]
fn relocation_width(kind: u32) -> Option<usize> {
    match kind {
        R_NONE => Some(0),
        257 | 260 | 1025 | 1026 | 1027 => Some(8),
        258 | 261 => Some(4),
        _ => None,
    }
}

#[cfg(target_arch = "x86_64")]
fn calculate_relocation(
    kind: u32,
    symbol_address: usize,
    addend: i64,
    load_bias: usize,
    place: usize,
) -> Result<Option<u64>, Error> {
    let symbol_value = symbol_address as i128;
    let addend_value = addend as i128;
    let bias_value = load_bias as i128;
    let place_value = place as i128;
    let value = match kind {
        R_NONE => return Ok(None),
        1 => unsigned_64(symbol_value + addend_value)?,
        2 | 4 => signed_32(symbol_value + addend_value - place_value)?,
        6 | 7 => unsigned_64(symbol_value)?,
        8 => unsigned_64(bias_value + addend_value)?,
        10 => unsigned_32(symbol_value + addend_value)?,
        11 => signed_32(symbol_value + addend_value)?,
        _ => return Err(Error::OpNotSupp),
    };
    Ok(Some(value))
}

#[cfg(target_arch = "aarch64")]
fn calculate_relocation(
    kind: u32,
    symbol_address: usize,
    addend: i64,
    load_bias: usize,
    place: usize,
) -> Result<Option<u64>, Error> {
    let symbol_value = symbol_address as i128;
    let addend_value = addend as i128;
    let bias_value = load_bias as i128;
    let place_value = place as i128;
    let value = match kind {
        R_NONE => return Ok(None),
        257 => unsigned_64(symbol_value + addend_value)?,
        258 => unsigned_32(symbol_value + addend_value)?,
        260 => signed_64(symbol_value + addend_value - place_value)?,
        261 => signed_32(symbol_value + addend_value - place_value)?,
        1025 | 1026 => unsigned_64(symbol_value + addend_value)?,
        1027 => unsigned_64(bias_value + addend_value)?,
        _ => return Err(Error::OpNotSupp),
    };
    Ok(Some(value))
}

fn unsigned_64(value: i128) -> Result<u64, Error> {
    if !(0..=u64::MAX as i128).contains(&value) {
        return Err(Error::Libbad);
    }
    Ok(value as u64)
}

fn unsigned_32(value: i128) -> Result<u64, Error> {
    if !(0..=u32::MAX as i128).contains(&value) {
        return Err(Error::Libbad);
    }
    Ok(value as u64)
}

fn signed_64(value: i128) -> Result<u64, Error> {
    if !(i64::MIN as i128..=i64::MAX as i128).contains(&value) {
        return Err(Error::Libbad);
    }
    Ok(value as i64 as u64)
}

fn signed_32(value: i128) -> Result<u64, Error> {
    if !(i32::MIN as i128..=i32::MAX as i128).contains(&value) {
        return Err(Error::Libbad);
    }
    Ok(value as i32 as u32 as u64)
}

fn validate_page_permissions(ranges: &[LoadRange]) -> Result<(), Error> {
    let mut previous_page_end = 0u64;
    let mut previous_permissions = 0u32;
    let mut has_previous = false;
    for range in ranges {
        let page_start = range.start & !((PAGE_SIZE as u64) - 1);
        let page_end = align_up(
            usize::try_from(range.end).map_err(|_| Error::Overflow)?,
            PAGE_SIZE,
        )
        .ok_or(Error::Overflow)? as u64;
        if has_previous && page_start < previous_page_end {
            if range.permissions != previous_permissions {
                return Err(Error::NoExec);
            }
            previous_page_end = previous_page_end.max(page_end);
        } else {
            previous_page_end = page_end;
            previous_permissions = range.permissions;
            has_previous = true;
        }
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
const fn native_machine() -> u16 {
    62
}

#[cfg(target_arch = "aarch64")]
const fn native_machine() -> u16 {
    183
}