/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::lib::alignment::align_up;
use crate::kernel::lib::endian;

use super::dl::{copy_file_range, protect_pages, read_exact, ObjectState, PAGE_SIZE};
use super::{
    AddressSpace, Error, FileSource, ObjectKind, Permissions, PermissionsApi as _, SymbolResolver,
};

const PE_OFFSET_FIELD: u64 = 0x3c;
const DOS_HEADER_SIZE: u64 = 0x40;
const COFF_HEADER_SIZE: usize = 20;
const OPTIONAL_HEADER_SIZE: usize = 4096;
const SECTION_HEADER_SIZE: usize = 40;
const MAX_SECTIONS: usize = 4096;
const MAX_IMAGE_SIZE: usize = 1 << 28;
const MAX_IMPORTS: usize = 1 << 18;
const MAX_EXPORTS: usize = 1 << 18;
const MAX_BASE_RELOCATIONS: usize = 1 << 18;
const MAX_NAME_SIZE: usize = 4096;
const MAX_TOTAL_NAME_BYTES: usize = 16 << 20;
const DIRECTORY_COUNT: usize = 16;

const IMAGE_FILE_EXECUTABLE_IMAGE: u16 = 0x0002;
const IMAGE_FILE_DLL: u16 = 0x2000;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const IMAGE_SCN_MEM_READ: u32 = 0x4000_0000;
const IMAGE_SCN_MEM_WRITE: u32 = 0x8000_0000;
const IMAGE_DIRECTORY_EXPORT: usize = 0;
const IMAGE_DIRECTORY_IMPORT: usize = 1;
const IMAGE_DIRECTORY_SECURITY: usize = 4;
const IMAGE_DIRECTORY_BASERELOC: usize = 5;
const IMAGE_DIRECTORY_TLS: usize = 9;
const IMAGE_DIRECTORY_DELAY_IMPORT: usize = 13;
const IMAGE_DIRECTORY_CLR: usize = 14;
const IMAGE_REL_BASED_ABSOLUTE: u16 = 0;
const IMAGE_REL_BASED_DIR64: u16 = 10;

#[cfg(target_arch = "x86_64")]
const NATIVE_MACHINE: u16 = 0x8664;
#[cfg(target_arch = "aarch64")]
const NATIVE_MACHINE: u16 = 0xaa64;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const NATIVE_MACHINE: u16 = 0;

pub(super) fn matches(magic: &[u8; 4]) -> bool {
    magic[0] == b'M' && magic[1] == b'Z'
}

#[derive(Clone, Copy, Default)]
struct DataDirectory {
    address: u32,
    size: u32,
}

#[derive(Clone, Copy)]
struct Section {
    virtual_address: u32,
    memory_size: u32,
    raw_offset: u32,
    raw_size: u32,
    characteristics: u32,
}

struct Import {
    target_rva: u32,
    dependency_index: usize,
    symbol: ImportSymbol,
}

enum ImportSymbol {
    //@FIXME: INDEXTYPE(Name); owned import-name bytes; Vec is an O(1)-indexed byte buffer, keep Vec.
    Name(Vec<u8>),
    Ordinal(u32),
}

struct Export {
    function_rva: u32,
    ordinal: u32,
    //@FIXME: INDEXTYPE(name); optional owned export-name bytes; Vec is an O(1)-indexed byte buffer, keep Vec.
    name: Option<Vec<u8>>,
}

struct ParsedPe<'a, F> {
    source: &'a F,
    kind: ObjectKind,
    image_base: u64,
    image_size: usize,
    mapping_length: usize,
    headers_size: u32,
    entry_rva: u32,
    //@FIXME: INDEXTYPE(sections); already sorted by virtual address; read_rva scans O(n), use partition_point/binary search on this Vec for O(log n) section selection.
    sections: Vec<Section>,
    directories: [DataDirectory; DIRECTORY_COUNT],
    //@FIXME: INDEXTYPE(imports); parse-order imports traversed by dependency index; keep Vec.
    imports: Vec<Import>,
    //@FIXME: INDEXTYPE(exports); name/ordinal lookup scans O(n); add hash side indexes for expected O(1) lookup if exports are numerous.
    exports: Vec<Export>,
    //@FIXME: INDEXTYPE(base_relocations); sorted relocation batch iterated in order; keep Vec.
    base_relocations: Vec<u32>,
    //@FIXME: INDEXTYPE(needed); dependency-order byte strings with O(1) ordinal access; keep Vec.
    needed: Vec<Vec<u8>>,
}

struct LoadedExport {
    ordinal: u32,
    //@FIXME: INDEXTYPE(name); optional owned export-name bytes; Vec is an O(1)-indexed byte buffer, keep Vec.
    name: Option<Vec<u8>>,
    address: usize,
}

pub(super) struct PeLoadedObject {
    kind: ObjectKind,
    mapping_base: usize,
    mapping_length: usize,
    load_bias: i128,
    entry: Option<usize>,
    image_base: u64,
    headers_size: u32,
    //@FIXME: INDEXTYPE(sections); loaded sections are iterated for permission checks; keep Vec unless replacing those scans with binary search is worthwhile.
    sections: Vec<Section>,
    //@FIXME: INDEXTYPE(imports); parse-order imports traversed by dependency index; keep Vec.
    imports: Vec<Import>,
    //@FIXME: INDEXTYPE(base_relocations); sorted relocation batch iterated in order; keep Vec.
    base_relocations: Vec<u32>,
    //@FIXME: INDEXTYPE(needed); dependency-order byte strings with O(1) ordinal access; keep Vec.
    needed: Vec<Vec<u8>>,
    //@FIXME: INDEXTYPE(exports); name/ordinal lookup scans O(n); add hash side indexes for expected O(1) lookup if exports are numerous.
    exports: Vec<LoadedExport>,
    state: ObjectState,
}

impl PeLoadedObject {
    pub(crate) fn load<F: FileSource, A: AddressSpace>(
        source: &F,
        address_space: &mut A,
        kind: ObjectKind,
        preferred_address: Option<usize>,
    ) -> Result<Self, Error> {
        let parsed = ParsedPe::parse(source, kind)?;
        let image_base = usize::try_from(parsed.image_base).map_err(|_| Error::Overflow)?;
        let can_relocate = !parsed.base_relocations.is_empty();
        let (preferred_address, fixed) = if can_relocate {
            (Some(preferred_address.unwrap_or(image_base)), false)
        } else {
            if preferred_address.is_some_and(|address| address != image_base) {
                return Err(Error::NoExec);
            }
            (Some(image_base), true)
        };
        let mapping_base = address_space.reserve(
            preferred_address,
            parsed.mapping_length,
            PAGE_SIZE,
            0,
            fixed,
        )?;
        if (fixed && mapping_base != image_base)
            || mapping_base % PAGE_SIZE != 0
            || mapping_base.checked_add(parsed.mapping_length).is_none()
        {
            address_space.release(mapping_base, parsed.mapping_length);
            return Err(Error::NoMem);
        }

        let mapping_length = parsed.mapping_length;
        let result = Self::load_reserved(parsed, address_space, mapping_base);
        if result.is_err() {
            address_space.release(mapping_base, mapping_length);
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
        self.load_bias
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
        self.exports
            .iter()
            .find(|symbol| symbol.name.as_deref() == Some(name))
            .map(|symbol| symbol.address)
    }

    pub(super) fn symbol_ordinal(&self, ordinal: u32) -> Option<usize> {
        self.exports
            .iter()
            .find(|symbol| symbol.ordinal == ordinal)
            .map(|symbol| symbol.address)
    }

    pub(crate) const fn symbol_size(&self, _name: &[u8]) -> Option<u64> {
        None
    }

    pub(super) fn for_each_exported_symbol<'a>(
        &'a self,
        visitor: &mut dyn FnMut(&'a [u8], usize, bool),
    ) {
        for symbol in &self.exports {
            if let Some(name) = &symbol.name {
                visitor(name, symbol.address, false);
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
        let delta = self.mapping_base as i128 - self.image_base as i128;
        let result = self.apply_relocations(address_space, resolver, delta);
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

        let header_pages = align_up(self.header_size(), PAGE_SIZE).ok_or(Error::Overflow)?;
        for page in &mut page_permissions[..header_pages / PAGE_SIZE] {
            *page |= Permissions::READ.bits();
        }
        for section in &self.sections {
            if section.memory_size == 0 {
                continue;
            }
            let start = section.virtual_address as usize & !(PAGE_SIZE - 1);
            let end = align_up(
                (section.virtual_address as usize)
                    .checked_add(section.memory_size as usize)
                    .ok_or(Error::Overflow)?,
                PAGE_SIZE,
            )
            .ok_or(Error::Overflow)?;
            let start_page = start / PAGE_SIZE;
            let end_page = end / PAGE_SIZE;
            let mut permissions = 0u8;
            if section.characteristics & IMAGE_SCN_MEM_READ != 0 {
                permissions |= Permissions::READ.bits();
            }
            if section.characteristics & IMAGE_SCN_MEM_WRITE != 0 {
                permissions |= Permissions::WRITE.bits();
            }
            if section.characteristics & IMAGE_SCN_MEM_EXECUTE != 0 {
                permissions |= Permissions::EXECUTE.bits();
            }
            for page in &mut page_permissions[start_page..end_page] {
                *page |= permissions;
            }
        }
        protect_pages(
            &mut self.state,
            address_space,
            self.mapping_base,
            self.mapping_length,
            &page_permissions,
        )
    }

    pub(crate) fn release<A: AddressSpace>(self, address_space: &mut A) {
        address_space.release(self.mapping_base, self.mapping_length);
    }

    fn header_size(&self) -> usize {
        self.headers_size as usize
    }

    fn apply_relocations<A: AddressSpace, R: SymbolResolver>(
        &self,
        address_space: &mut A,
        resolver: &mut R,
        delta: i128,
    ) -> Result<(), Error> {
        if delta != 0 {
            for rva in &self.base_relocations {
                let target = self
                    .mapping_base
                    .checked_add(*rva as usize)
                    .ok_or(Error::Overflow)?;
                let mut bytes = [0u8; 8];
                address_space.read(target, &mut bytes)?;
                let original = u64::from_le_bytes(bytes) as i128;
                let relocated = original.checked_add(delta).ok_or(Error::Overflow)?;
                if !(0..=u64::MAX as i128).contains(&relocated) {
                    return Err(Error::Libbad);
                }
                address_space.write(target, &(relocated as u64).to_le_bytes())?;
            }
        }
        for import in &self.imports {
            let dependency = self
                .needed
                .get(import.dependency_index)
                .ok_or(Error::NoExec)?;
            let resolved = match &import.symbol {
                ImportSymbol::Name(name) => resolver.resolve_in(dependency, name),
                ImportSymbol::Ordinal(ordinal) => {
                    resolver.resolve_ordinal_in(dependency, *ordinal)
                }
            }
            .ok_or(Error::NoExec)?;
            let address = u64::try_from(resolved.address).map_err(|_| Error::Overflow)?;
            let target = self
                .mapping_base
                .checked_add(import.target_rva as usize)
                .ok_or(Error::Overflow)?;
            address_space.write(target, &address.to_le_bytes())?;
        }
        Ok(())
    }

    fn load_reserved<F: FileSource, A: AddressSpace>(
        parsed: ParsedPe<'_, F>,
        address_space: &mut A,
        mapping_base: usize,
    ) -> Result<Self, Error> {
        address_space.zero(mapping_base, parsed.mapping_length)?;
        copy_file_range(
            parsed.source,
            address_space,
            0,
            mapping_base,
            parsed.headers_size as u64,
        )?;
        for section in &parsed.sections {
            if section.raw_size == 0 {
                continue;
            }
            let target = mapping_base
                .checked_add(section.virtual_address as usize)
                .ok_or(Error::Overflow)?;
            copy_file_range(
                parsed.source,
                address_space,
                section.raw_offset as u64,
                target,
                section.raw_size as u64,
            )?;
        }

        let delta = mapping_base as i128 - parsed.image_base as i128;
        if delta != 0 && parsed.base_relocations.is_empty() {
            return Err(Error::NoExec);
        }
        let mut exports = Vec::new();
        exports
            .try_reserve_exact(parsed.exports.len())
            .map_err(|_| Error::NoMem)?;
        for export in parsed.exports {
            let address = mapping_base
                .checked_add(export.function_rva as usize)
                .ok_or(Error::Overflow)?;
            exports.push(LoadedExport {
                ordinal: export.ordinal,
                name: export.name,
                address,
            });
        }
        let entry = if parsed.kind == ObjectKind::Executable {
            Some(
                mapping_base
                    .checked_add(parsed.entry_rva as usize)
                    .ok_or(Error::Overflow)?,
            )
        } else {
            None
        };
        let pending = !parsed.imports.is_empty() || (delta != 0 && !parsed.base_relocations.is_empty());
        Ok(Self {
            kind: parsed.kind,
            mapping_base,
            mapping_length: parsed.mapping_length,
            load_bias: mapping_base as i128 - parsed.image_base as i128,
            entry,
            image_base: parsed.image_base,
            headers_size: parsed.headers_size,
            sections: parsed.sections,
            imports: parsed.imports,
            base_relocations: parsed.base_relocations,
            needed: parsed.needed,
            exports,
            state: if pending {
                ObjectState::RelocationPending
            } else {
                ObjectState::ReadyToSeal
            },
        })
    }
}

impl<'a, F: FileSource> ParsedPe<'a, F> {
    fn parse(source: &'a F, kind: ObjectKind) -> Result<Self, Error> {
        if source.len() < DOS_HEADER_SIZE {
            return Err(Error::NoExec);
        }
        let mut dos_header = [0u8; DOS_HEADER_SIZE as usize];
        read_exact(source, 0, &mut dos_header)?;
        if dos_header[..2] != *b"MZ" {
            return Err(Error::NoExec);
        }
        let nt_offset = u32::from_le_bytes([
            dos_header[0x3c],
            dos_header[0x3d],
            dos_header[0x3e],
            dos_header[0x3f],
        ]) as u64;
        if nt_offset < DOS_HEADER_SIZE {
            return Err(Error::NoExec);
        }
        let coff_offset = nt_offset.checked_add(4).ok_or(Error::Overflow)?;
        let coff_end = coff_offset
            .checked_add(COFF_HEADER_SIZE as u64)
            .ok_or(Error::Overflow)?;
        if coff_end > source.len() {
            return Err(Error::NoExec);
        }
        let mut nt_header = [0u8; 4 + COFF_HEADER_SIZE];
        read_exact(source, nt_offset, &mut nt_header)?;
        if nt_header[..4] != *b"PE\0\0" {
            return Err(Error::NoExec);
        }
        let machine = endian::read_u16(&nt_header, 4).ok_or(Error::NoExec)?;
        if machine != NATIVE_MACHINE {
            return Err(Error::OpNotSupp);
        }
        let section_count = endian::read_u16(&nt_header, 6).ok_or(Error::NoExec)? as usize;
        let optional_size = endian::read_u16(&nt_header, 20).ok_or(Error::NoExec)? as usize;
        let characteristics = endian::read_u16(&nt_header, 22).ok_or(Error::NoExec)?;
        if section_count == 0 || section_count > MAX_SECTIONS {
            return Err(Error::NoExec);
        }
        if characteristics & IMAGE_FILE_EXECUTABLE_IMAGE == 0 {
            return Err(Error::NoExec);
        }
        match (kind, characteristics & IMAGE_FILE_DLL != 0) {
            (ObjectKind::Executable, false) | (ObjectKind::SharedObject, true) => {}
            _ => return Err(Error::NoExec),
        }
        if optional_size < 112 || optional_size > OPTIONAL_HEADER_SIZE {
            return Err(Error::NoExec);
        }
        let optional_offset = coff_end;
        let optional_end = optional_offset
            .checked_add(optional_size as u64)
            .ok_or(Error::Overflow)?;
        if optional_end > source.len() {
            return Err(Error::NoExec);
        }
        let mut optional_header = [0u8; OPTIONAL_HEADER_SIZE];
        read_exact(
            source,
            optional_offset,
            &mut optional_header[..optional_size],
        )?;
        if endian::read_u16(&optional_header, 0).ok_or(Error::NoExec)? != 0x20b {
            return Err(Error::OpNotSupp);
        }
        let entry_rva = endian::read_u32(&optional_header, 16).ok_or(Error::NoExec)?;
        let image_base = endian::read_u64(&optional_header, 24).ok_or(Error::NoExec)?;
        let section_alignment = endian::read_u32(&optional_header, 32).ok_or(Error::NoExec)?;
        let file_alignment = endian::read_u32(&optional_header, 36).ok_or(Error::NoExec)?;
        let image_size = endian::read_u32(&optional_header, 56).ok_or(Error::NoExec)? as usize;
        let headers_size = endian::read_u32(&optional_header, 60).ok_or(Error::NoExec)?;
        let directory_count = endian::read_u32(&optional_header, 108).ok_or(Error::NoExec)? as usize;
        if directory_count > DIRECTORY_COUNT {
            return Err(Error::OpNotSupp);
        }
        let directory_bytes = directory_count
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(112))
            .ok_or(Error::Overflow)?;
        if directory_bytes > optional_size {
            return Err(Error::NoExec);
        }
        if image_size == 0
            || image_size > MAX_IMAGE_SIZE
            || image_size % PAGE_SIZE != 0
            || image_base % 0x10000 != 0
            || !section_alignment.is_power_of_two()
            || section_alignment < PAGE_SIZE as u32
            || !file_alignment.is_power_of_two()
            || !(512..=65_536).contains(&file_alignment)
            || file_alignment > section_alignment
            || image_size % section_alignment as usize != 0
            || headers_size == 0
            || headers_size as usize > image_size
            || headers_size as u64 > source.len()
            || headers_size % file_alignment != 0
        {
            return Err(Error::NoExec);
        }
        image_base
            .checked_add(image_size as u64)
            .ok_or(Error::Overflow)?;
        let mapping_length = align_up(image_size, PAGE_SIZE).ok_or(Error::Overflow)?;

        let mut directories = [DataDirectory::default(); DIRECTORY_COUNT];
        for (index, directory) in directories.iter_mut().enumerate().take(directory_count) {
            let offset = 112 + index * 8;
            directory.address = endian::read_u32(&optional_header, offset).ok_or(Error::NoExec)?;
            directory.size = endian::read_u32(&optional_header, offset + 4).ok_or(Error::NoExec)?;
            if (directory.address == 0) != (directory.size == 0) {
                return Err(Error::NoExec);
            }
            if directory.address == 0 {
                continue;
            }
            let end = directory.address as u64 + directory.size as u64;
            if index == IMAGE_DIRECTORY_SECURITY {
                if end > source.len() {
                    return Err(Error::NoExec);
                }
            } else if end > image_size as u64 {
                return Err(Error::NoExec);
            }
        }
        if [
            IMAGE_DIRECTORY_TLS,
            IMAGE_DIRECTORY_DELAY_IMPORT,
            IMAGE_DIRECTORY_CLR,
        ]
        .iter()
        .any(|index| directories[*index].address != 0)
        {
            return Err(Error::OpNotSupp);
        }

        let section_table_offset = optional_end;
        let section_table_size = section_count
            .checked_mul(SECTION_HEADER_SIZE)
            .ok_or(Error::Overflow)?;
        let section_table_end = section_table_offset
            .checked_add(section_table_size as u64)
            .ok_or(Error::Overflow)?;
        if section_table_end > source.len() || section_table_end > headers_size as u64 {
            return Err(Error::NoExec);
        }
        let mut sections = Vec::new();
        let mut raw_ranges = Vec::new();
        sections
            .try_reserve_exact(section_count)
            .map_err(|_| Error::NoMem)?;
        raw_ranges
            .try_reserve_exact(section_count)
            .map_err(|_| Error::NoMem)?;
        for index in 0..section_count {
            let offset = section_table_offset
                .checked_add((index * SECTION_HEADER_SIZE) as u64)
                .ok_or(Error::Overflow)?;
            let mut bytes = [0u8; SECTION_HEADER_SIZE];
            read_exact(source, offset, &mut bytes)?;
            let virtual_size = endian::read_u32(&bytes, 8).ok_or(Error::NoExec)?;
            let virtual_address = endian::read_u32(&bytes, 12).ok_or(Error::NoExec)?;
            let raw_size = endian::read_u32(&bytes, 16).ok_or(Error::NoExec)?;
            let raw_offset = endian::read_u32(&bytes, 20).ok_or(Error::NoExec)?;
            let section = Section {
                virtual_address,
                memory_size: virtual_size.max(raw_size),
                raw_offset,
                raw_size,
                characteristics: endian::read_u32(&bytes, 36).ok_or(Error::NoExec)?,
            };
            if section.memory_size == 0 {
                continue;
            }
            let virtual_end = section.virtual_address as u64 + section.memory_size as u64;
            if section.virtual_address < headers_size
                || section.virtual_address % section_alignment != 0
                || virtual_end > image_size as u64
            {
                return Err(Error::NoExec);
            }
            if section.raw_size != 0 {
                let raw_end = section.raw_offset as u64 + section.raw_size as u64;
                if section.raw_offset < headers_size
                    || section.raw_offset % file_alignment != 0
                    || section.raw_size % file_alignment != 0
                    || raw_end > source.len()
                {
                    return Err(Error::NoExec);
                }
                raw_ranges.push((section.raw_offset as u64, raw_end));
            }
            sections.push(section);
        }
        raw_ranges.sort_unstable_by_key(|range| range.0);
        if raw_ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(Error::NoExec);
        }
        sections.sort_unstable_by_key(|section| section.virtual_address);
        if sections.windows(2).any(|pair| {
            pair[0].virtual_address as u64 + pair[0].memory_size as u64
                > pair[1].virtual_address as u64
        }) {
            return Err(Error::NoExec);
        }
        if kind == ObjectKind::Executable
            && (entry_rva == 0 || !sections.iter().any(|section| {
                section.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
                    && entry_rva >= section.virtual_address
                    && (entry_rva as u64)
                        < section.virtual_address as u64 + section.memory_size as u64
            }))
        {
            return Err(Error::NoExec);
        }
        if kind == ObjectKind::SharedObject && entry_rva != 0 {
            return Err(Error::OpNotSupp);
        }

        let mut parsed = Self {
            source,
            kind,
            image_base,
            image_size,
            mapping_length,
            headers_size,
            entry_rva,
            sections,
            directories,
            imports: Vec::new(),
            exports: Vec::new(),
            base_relocations: Vec::new(),
            needed: Vec::new(),
        };
        parsed.parse_exports()?;
        parsed.parse_imports()?;
        parsed.parse_base_relocations()?;
        parsed.base_relocations.sort_unstable();
        if parsed
            .base_relocations
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(Error::Libbad);
        }
        Ok(parsed)
    }

    fn read_rva(&self, rva: u32, buffer: &mut [u8]) -> Result<(), Error> {
        if buffer.is_empty() {
            return Ok(());
        }
        let end = (rva as u64)
            .checked_add(buffer.len() as u64)
            .ok_or(Error::Overflow)?;
        if end > self.image_size as u64 {
            return Err(Error::NoExec);
        }
        if rva < self.headers_size && end <= self.headers_size as u64 {
            return read_exact(self.source, rva as u64, buffer);
        }
        for section in &self.sections {
            let start = section.virtual_address as u64;
            let raw_end = start + section.raw_size as u64;
            if rva as u64 >= start && end <= raw_end {
                let offset = section.raw_offset as u64 + rva as u64 - start;
                return read_exact(self.source, offset, buffer);
            }
        }
        Err(Error::NoExec)
    }

    fn read_u16_rva(&self, rva: u32) -> Result<u16, Error> {
        let mut bytes = [0u8; 2];
        self.read_rva(rva, &mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32_rva(&self, rva: u32) -> Result<u32, Error> {
        let mut bytes = [0u8; 4];
        self.read_rva(rva, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_u64_rva(&self, rva: u32) -> Result<u64, Error> {
        let mut bytes = [0u8; 8];
        self.read_rva(rva, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn read_string_rva(&self, rva: u32) -> Result<Vec<u8>, Error> {
        let mut output = Vec::new();
        for index in 0..MAX_NAME_SIZE {
            let current = rva
                .checked_add(u32::try_from(index).map_err(|_| Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            let mut byte = [0u8; 1];
            self.read_rva(current, &mut byte)?;
            if byte[0] == 0 {
                return Ok(output);
            }
            output.try_reserve(1).map_err(|_| Error::NoMem)?;
            output.push(byte[0]);
        }
        Err(Error::Fbig)
    }

    fn parse_exports(&mut self) -> Result<(), Error> {
        let directory = self.directories[IMAGE_DIRECTORY_EXPORT];
        if directory.address == 0 {
            return Ok(());
        }
        if directory.size < 40 {
            return Err(Error::NoExec);
        }
        let mut bytes = [0u8; 40];
        self.read_rva(directory.address, &mut bytes)?;
        let ordinal_base = endian::read_u32(&bytes, 16).ok_or(Error::NoExec)?;
        let function_count = endian::read_u32(&bytes, 20).ok_or(Error::NoExec)? as usize;
        let name_count = endian::read_u32(&bytes, 24).ok_or(Error::NoExec)? as usize;
        let functions_rva = endian::read_u32(&bytes, 28).ok_or(Error::NoExec)?;
        let names_rva = endian::read_u32(&bytes, 32).ok_or(Error::NoExec)?;
        let ordinals_rva = endian::read_u32(&bytes, 36).ok_or(Error::NoExec)?;
        if function_count > MAX_EXPORTS || name_count > MAX_EXPORTS {
            return Err(Error::Fbig);
        }
        if function_count != 0 {
            ordinal_base
                .checked_add(
                    u32::try_from(function_count - 1).map_err(|_| Error::Overflow)?,
                )
                .ok_or(Error::Overflow)?;
        }
        let export_capacity = function_count
            .checked_add(name_count)
            .ok_or(Error::Overflow)?;
        self.exports
            .try_reserve_exact(export_capacity)
            .map_err(|_| Error::NoMem)?;
        let mut function_rvas = Vec::new();
        function_rvas
            .try_reserve_exact(function_count)
            .map_err(|_| Error::NoMem)?;
        for index in 0..function_count {
            let index_u32 = u32::try_from(index).map_err(|_| Error::Overflow)?;
            let function_rva = self.read_u32_rva(
                functions_rva
                    .checked_add(index_u32.checked_mul(4).ok_or(Error::Overflow)?)
                    .ok_or(Error::Overflow)?,
            )?;
            function_rvas.push(function_rva);
        }
        let mut has_name = Vec::new();
        has_name
            .try_reserve_exact(function_count)
            .map_err(|_| Error::NoMem)?;
        has_name.resize(function_count, false);
        let mut total_name_bytes = 0usize;
        for index in 0..name_count {
            let index_u32 = u32::try_from(index).map_err(|_| Error::Overflow)?;
            let name_rva = self.read_u32_rva(
                names_rva
                    .checked_add(index_u32.checked_mul(4).ok_or(Error::Overflow)?)
                    .ok_or(Error::Overflow)?,
            )?;
            let function_index = self.read_u16_rva(
                ordinals_rva
                    .checked_add(index_u32.checked_mul(2).ok_or(Error::Overflow)?)
                    .ok_or(Error::Overflow)?,
            )? as usize;
            if function_index >= function_count {
                return Err(Error::NoExec);
            }
            let function_rva = function_rvas[function_index];
            if function_rva == 0 {
                return Err(Error::NoExec);
            }
            if function_rva >= directory.address
                && function_rva
                    < directory
                        .address
                        .checked_add(directory.size)
                        .ok_or(Error::Overflow)?
            {
                return Err(Error::OpNotSupp);
            }
            if !self.contains_rva(function_rva, 1) {
                return Err(Error::NoExec);
            }
            let name = self.read_string_rva(name_rva)?;
            if name.is_empty() {
                return Err(Error::NoExec);
            }
            total_name_bytes = total_name_bytes
                .checked_add(name.len())
                .ok_or(Error::Overflow)?;
            if total_name_bytes > MAX_TOTAL_NAME_BYTES {
                return Err(Error::Fbig);
            }
            let ordinal = ordinal_base
                .checked_add(u32::try_from(function_index).map_err(|_| Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            self.exports.push(Export {
                function_rva,
                ordinal,
                name: Some(name),
            });
            has_name[function_index] = true;
        }
        let directory_end = directory
            .address
            .checked_add(directory.size)
            .ok_or(Error::Overflow)?;
        for (index, function_rva) in function_rvas.into_iter().enumerate() {
            let index_u32 = u32::try_from(index).map_err(|_| Error::Overflow)?;
            if function_rva == 0 {
                continue;
            }
            if function_rva >= directory.address && function_rva < directory_end {
                return Err(Error::OpNotSupp);
            }
            if !self.contains_rva(function_rva, 1) {
                return Err(Error::NoExec);
            }
            let ordinal = ordinal_base
                .checked_add(index_u32)
                .ok_or(Error::Overflow)?;
            if !has_name[index] {
                self.exports.push(Export {
                    function_rva,
                    ordinal,
                    name: None,
                });
            }
        }
        Ok(())
    }

    fn parse_imports(&mut self) -> Result<(), Error> {
        let directory = self.directories[IMAGE_DIRECTORY_IMPORT];
        if directory.address == 0 {
            return Ok(());
        }
        if directory.size == 0 || directory.size % 20 != 0 {
            return Err(Error::NoExec);
        }
        let descriptor_count = directory.size as usize / 20;
        if descriptor_count > 4096 {
            return Err(Error::Fbig);
        }
        let mut terminated = false;
        for index in 0..descriptor_count {
            let descriptor_rva = directory
                .address
                .checked_add(u32::try_from(index.checked_mul(20).ok_or(Error::Overflow)?)
                    .map_err(|_| Error::Overflow)?)
                .ok_or(Error::Overflow)?;
            let mut bytes = [0u8; 20];
            self.read_rva(descriptor_rva, &mut bytes)?;
            if bytes == [0; 20] {
                terminated = true;
                break;
            }
            let original_thunk = endian::read_u32(&bytes, 0).ok_or(Error::NoExec)?;
            let name_rva = endian::read_u32(&bytes, 12).ok_or(Error::NoExec)?;
            let first_thunk = endian::read_u32(&bytes, 16).ok_or(Error::NoExec)?;
            if name_rva == 0 || first_thunk == 0 {
                return Err(Error::NoExec);
            }
            let library_name = self.read_string_rva(name_rva)?;
            if library_name.is_empty() {
                return Err(Error::NoExec);
            }
            let dependency_index = if let Some(index) = self
                .needed
                .iter()
                .position(|name| *name == library_name)
            {
                index
            } else {
                self.needed.try_reserve(1).map_err(|_| Error::NoMem)?;
                self.needed.push(library_name);
                self.needed.len() - 1
            };
            let lookup_table = if original_thunk == 0 {
                first_thunk
            } else {
                original_thunk
            };
            let mut thunk_terminated = false;
            for thunk_index in 0..MAX_IMPORTS {
                let thunk_offset = u32::try_from(
                    thunk_index.checked_mul(8).ok_or(Error::Overflow)?,
                )
                .map_err(|_| Error::Overflow)?;
                let lookup_rva = lookup_table.checked_add(thunk_offset).ok_or(Error::Overflow)?;
                let thunk = self.read_u64_rva(lookup_rva)?;
                if thunk == 0 {
                    thunk_terminated = true;
                    break;
                }
                let symbol = if thunk & (1u64 << 63) != 0 {
                    if thunk & 0x7fff_ffff_ffff_0000 != 0 {
                        return Err(Error::NoExec);
                    }
                    let ordinal = (thunk & 0xffff) as u32;
                    if ordinal == 0 {
                        return Err(Error::NoExec);
                    }
                    ImportSymbol::Ordinal(ordinal)
                } else {
                    let import_name_rva = u32::try_from(thunk).map_err(|_| Error::NoExec)?;
                    let import_name_rva = import_name_rva.checked_add(2).ok_or(Error::Overflow)?;
                    let name = self.read_string_rva(import_name_rva)?;
                    if name.is_empty() {
                        return Err(Error::NoExec);
                    }
                    ImportSymbol::Name(name)
                };
                let target_rva = first_thunk
                    .checked_add(thunk_offset)
                    .ok_or(Error::Overflow)?;
                if !self.is_writable_rva(target_rva, 8) {
                    return Err(Error::Libbad);
                }
                if self.imports.len() >= MAX_IMPORTS {
                    return Err(Error::Fbig);
                }
                self.imports.try_reserve(1).map_err(|_| Error::NoMem)?;
                self.imports.push(Import {
                    target_rva,
                    dependency_index,
                    symbol,
                });
            }
            if !thunk_terminated {
                return Err(Error::Fbig);
            }
        }
        if !terminated {
            return Err(Error::NoExec);
        }
        Ok(())
    }

    fn parse_base_relocations(&mut self) -> Result<(), Error> {
        let directory = self.directories[IMAGE_DIRECTORY_BASERELOC];
        if directory.address == 0 {
            return Ok(());
        }
        let mut consumed = 0u32;
        while consumed < directory.size {
            if directory.size - consumed < 8 {
                return Err(Error::NoExec);
            }
            let block_rva = directory
                .address
                .checked_add(consumed)
                .ok_or(Error::Overflow)?;
            let mut header = [0u8; 8];
            self.read_rva(block_rva, &mut header)?;
            let page_rva = endian::read_u32(&header, 0).ok_or(Error::NoExec)?;
            let block_size = endian::read_u32(&header, 4).ok_or(Error::NoExec)?;
            if page_rva % PAGE_SIZE as u32 != 0
                || block_size < 8
                || block_size % 2 != 0
                || block_size > directory.size - consumed
            {
                return Err(Error::NoExec);
            }
            let entry_count = (block_size as usize - 8) / 2;
            for index in 0..entry_count {
                let offset = u32::try_from(index.checked_mul(2).ok_or(Error::Overflow)?)
                    .map_err(|_| Error::Overflow)?;
                let entry_rva = block_rva
                    .checked_add(8)
                    .and_then(|value| value.checked_add(offset))
                    .ok_or(Error::Overflow)?;
                let entry = self.read_u16_rva(entry_rva)?;
                let kind = entry >> 12;
                let displacement = (entry & 0x0fff) as u32;
                if kind == IMAGE_REL_BASED_ABSOLUTE {
                    continue;
                }
                if kind != IMAGE_REL_BASED_DIR64 {
                    return Err(Error::OpNotSupp);
                }
                let target = page_rva.checked_add(displacement).ok_or(Error::Overflow)?;
                if !self.contains_rva(target, 8)
                    || self.base_relocations.len() >= MAX_BASE_RELOCATIONS
                {
                    return Err(Error::NoExec);
                }
                self.base_relocations
                    .try_reserve(1)
                    .map_err(|_| Error::NoMem)?;
                self.base_relocations.push(target);
            }
            consumed = consumed.checked_add(block_size).ok_or(Error::Overflow)?;
        }
        Ok(())
    }

    fn is_writable_rva(&self, rva: u32, length: usize) -> bool {
        self.sections.iter().any(|section| {
            section.characteristics & IMAGE_SCN_MEM_WRITE != 0
                && section_contains(section, rva, length)
        })
    }

    fn contains_rva(&self, rva: u32, length: usize) -> bool {
        self.sections
            .iter()
            .any(|section| section_contains(section, rva, length))
    }
}

fn section_contains(section: &Section, rva: u32, length: usize) -> bool {
    let Some(end) = (rva as u64).checked_add(length as u64) else {
        return false;
    };
    rva >= section.virtual_address
        && end <= section.virtual_address as u64 + section.memory_size as u64
}

pub(super) fn load<F: FileSource, A: AddressSpace>(
    source: &F,
    address_space: &mut A,
    kind: ObjectKind,
    preferred_address: Option<usize>,
) -> Result<super::dl::LoadedObjectState, Error> {
    let object = PeLoadedObject::load(source, address_space, kind, preferred_address)?;
    Ok(super::dl::LoadedObjectState::Pe(object))
}

#[cfg(test)]
#[path = "../../tests/kernel/dl/pe.rs"]
mod tests;
