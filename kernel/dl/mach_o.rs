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

const MH_MAGIC_64_LITTLE_ENDIAN: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];
const MACH_HEADER_64_SIZE: usize = 32;
const LOAD_COMMAND_SIZE: usize = 8;
const SEGMENT_64_SIZE: usize = 72;
const SECTION_64_SIZE: usize = 80;
const MAX_LOAD_COMMANDS: usize = 65_536;
const MAX_LOAD_COMMAND_BYTES: usize = 16 << 20;
const MAX_SEGMENTS: usize = 4096;
const MAX_IMAGE_SIZE: usize = 1 << 28;
const MAX_RELOCATIONS: usize = 1 << 18;
const MAX_SYMBOLS: usize = 1 << 18;
const MAX_SYMBOL_NAME_SIZE: usize = 4096;
const MAX_LINKEDIT_DATA_SIZE: usize = 16 << 20;

const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const MH_EXECUTE: u32 = 2;
const MH_DYLIB: u32 = 6;
const MH_BUNDLE: u32 = 8;
const MH_PIE: u32 = 0x0020_0000;
const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_UNIXTHREAD: u32 = 0x5;
const LC_ROUTINES: u32 = 0x11;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_LOAD_DYLINKER: u32 = 0xe;
const LC_ID_DYLIB: u32 = 0xd;
const LC_ROUTINES_64: u32 = 0x1a;
const LC_DYLD_INFO: u32 = 0x22;
const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;
const LC_MAIN: u32 = 0x8000_0028;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
const LC_DYLD_EXPORTS_TRIE: u32 = 0x8000_0033;
const LC_DYLD_CHAINED_FIXUPS: u32 = 0x8000_0034;
const LC_RPATH: u32 = 0x8000_001c;
const LC_REQ_DYLD: u32 = 0x8000_0000;

const VM_PROT_READ: u32 = 1;
const VM_PROT_WRITE: u32 = 2;
const VM_PROT_EXECUTE: u32 = 4;
const BIND_TYPE_POINTER: u8 = 1;
const BIND_SYMBOL_FLAGS_WEAK_IMPORT: u8 = 1;
const EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION: u64 = 0x04;
const EXPORT_SYMBOL_FLAGS_REEXPORT: u64 = 0x08;
const EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER: u64 = 0x10;
const EXPORT_SYMBOL_FLAGS_KIND_MASK: u64 = 0x03;
const EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE: u64 = 0x02;
const EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL: u64 = 0x01;
const N_STAB: u8 = 0xe0;
const N_TYPE: u8 = 0x0e;
const N_EXT: u8 = 0x01;
const N_PEXT: u8 = 0x10;
const N_UNDF: u8 = 0x00;
const N_ABS: u8 = 0x02;
const N_SECT: u8 = 0x0e;
const N_WEAK_DEF: u16 = 0x0080;
const S_MOD_INIT_FUNC_POINTERS: u32 = 0x9;
const S_MOD_TERM_FUNC_POINTERS: u32 = 0xa;

#[cfg(target_arch = "x86_64")]
const NATIVE_CPU_TYPE: u32 = CPU_TYPE_X86_64;
#[cfg(target_arch = "aarch64")]
const NATIVE_CPU_TYPE: u32 = CPU_TYPE_ARM64;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const NATIVE_CPU_TYPE: u32 = 0;

pub(super) fn matches(magic: &[u8; 4]) -> bool {
    matches!(
        *magic,
        [0xfe, 0xed, 0xfa, 0xce]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [0xbe, 0xba, 0xfe, 0xca]
            | [0xca, 0xfe, 0xba, 0xbf]
            | [0xbf, 0xba, 0xfe, 0xca]
    )
}

#[derive(Clone, Copy)]
struct Segment {
    virtual_address: u64,
    virtual_size: u64,
    file_offset: u64,
    file_size: u64,
    initial_protection: u32,
    maximum_protection: u32,
    mapped: bool,
    linkedit: bool,
}

#[derive(Clone, Copy, Default)]
struct FileRange {
    offset: u32,
    size: u32,
}

#[derive(Clone, Copy, Default)]
struct SymbolTable {
    symbol_offset: u32,
    symbol_count: u32,
    string_offset: u32,
    string_size: u32,
}

#[derive(Clone, Copy, Default)]
struct DyldInfo {
    rebase: FileRange,
    bind: FileRange,
    weak_bind: FileRange,
    lazy_bind: FileRange,
    exports: FileRange,
}

#[derive(Clone, Copy)]
enum ExportAddress {
    Relative(u64),
    Absolute(u64),
}

struct ParsedExport {
    name: Vec<u8>,
    address: ExportAddress,
    weak: bool,
}

struct Binding {
    virtual_address: u64,
    name: Vec<u8>,
    library_ordinal: i32,
    addend: i64,
    weak: bool,
}

struct LoadedExport {
    name: Vec<u8>,
    address: usize,
    weak: bool,
}

struct ParsedMachO<'a, F> {
    source: &'a F,
    kind: ObjectKind,
    minimum_page: usize,
    mapping_length: usize,
    fixed_address: bool,
    segments: Vec<Segment>,
    entry_virtual_address: Option<u64>,
    bindings: Vec<Binding>,
    rebases: Vec<u64>,
    exports: Vec<ParsedExport>,
    needed: Vec<Vec<u8>>,
    needed_weak: Vec<bool>,
    rpaths: Vec<Vec<u8>>,
}

pub(super) struct MachOLoadedObject {
    kind: ObjectKind,
    mapping_base: usize,
    mapping_length: usize,
    load_bias: i128,
    entry: Option<usize>,
    segments: Vec<Segment>,
    bindings: Vec<Binding>,
    rebases: Vec<u64>,
    exports: Vec<LoadedExport>,
    needed: Vec<Vec<u8>>,
    needed_weak: Vec<bool>,
    rpaths: Vec<Vec<u8>>,
    state: ObjectState,
}

impl MachOLoadedObject {
    pub(crate) fn load<F: FileSource, A: AddressSpace>(
        source: &F,
        address_space: &mut A,
        kind: ObjectKind,
        preferred_address: Option<usize>,
    ) -> Result<Self, Error> {
        let parsed = ParsedMachO::parse(source, kind)?;
        let preferred = if parsed.fixed_address {
            if preferred_address.is_some_and(|address| address != parsed.minimum_page) {
                return Err(Error::NoExec);
            }
            Some(parsed.minimum_page)
        } else {
            preferred_address.or(Some(parsed.minimum_page))
        };
        let mapping_base = address_space.reserve(
            preferred,
            parsed.mapping_length,
            PAGE_SIZE,
            0,
            parsed.fixed_address,
        )?;
        if mapping_base % PAGE_SIZE != 0
            || (parsed.fixed_address && mapping_base != parsed.minimum_page)
            || mapping_base.checked_add(parsed.mapping_length).is_none()
        {
            address_space.release(mapping_base, parsed.mapping_length);
            return Err(Error::NoMem);
        }
        let load_bias = mapping_base as i128 - parsed.minimum_page as i128;

        let mapping_length = parsed.mapping_length;
        let result = Self::load_reserved(parsed, address_space, mapping_base, load_bias);
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

    pub(crate) fn search_paths(&self) -> &[Vec<u8>] {
        &self.rpaths
    }

    pub(crate) fn dependency_is_weak(&self, index: usize) -> bool {
        self.needed_weak.get(index).copied().unwrap_or(false)
    }

    pub(crate) fn symbol(&self, name: &[u8]) -> Option<usize> {
        self.exports
            .iter()
            .find(|symbol| symbol.name == name)
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
            visitor(&symbol.name, symbol.address, symbol.weak);
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
        for segment in &self.segments {
            if !segment.mapped || segment.virtual_size == 0 {
                continue;
            }
            let first_page = segment.virtual_address as usize & !(PAGE_SIZE - 1);
            let end_address = segment
                .virtual_address
                .checked_add(segment.virtual_size)
                .ok_or(Error::Overflow)?;
            let end_page = align_up(usize::try_from(end_address).map_err(|_| Error::Overflow)?, PAGE_SIZE)
                .ok_or(Error::Overflow)?;
            let start_index = self.page_index(first_page)?;
            let end_index = self.page_index(end_page)?;
            let mut permissions = 0u8;
            if segment.initial_protection & VM_PROT_READ != 0 {
                permissions |= Permissions::READ.bits();
            }
            if segment.initial_protection & VM_PROT_WRITE != 0 {
                permissions |= Permissions::WRITE.bits();
            }
            if segment.initial_protection & VM_PROT_EXECUTE != 0 {
                permissions |= Permissions::EXECUTE.bits();
            }
            for page in &mut page_permissions[start_index..end_index] {
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

    fn page_index(&self, virtual_page: usize) -> Result<usize, Error> {
        let runtime_page = usize::try_from(self.load_bias + virtual_page as i128)
            .map_err(|_| Error::Overflow)?;
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
        let slide = self.load_bias as i128;
        if slide != 0 {
            for virtual_address in &self.rebases {
                let target = self.runtime_address(*virtual_address)?;
                let mut bytes = [0u8; 8];
                address_space.read(target, &mut bytes)?;
                let original = u64::from_le_bytes(bytes) as i128;
                let rebased = original.checked_add(slide).ok_or(Error::Overflow)?;
                if !(0..=u64::MAX as i128).contains(&rebased) {
                    return Err(Error::Libbad);
                }
                address_space.write(target, &(rebased as u64).to_le_bytes())?;
            }
        }
        for binding in &self.bindings {
            let resolved = match binding.library_ordinal {
                0 => self
                    .symbol(&binding.name)
                    .map(|address| super::ResolvedSymbol { address }),
                -1 => None,
                -2 => resolver.resolve(&binding.name),
                -3 => resolver.resolve_weak(&binding.name),
                ordinal if ordinal > 0 => self
                    .needed
                    .get((ordinal - 1) as usize)
                    .and_then(|library| resolver.resolve_in(library, &binding.name)),
                _ => None,
            };
            let address = match resolved {
                Some(symbol) => symbol.address as i128,
                None
                    if binding.weak
                        || (binding.library_ordinal > 0
                            && self.dependency_is_weak(
                                (binding.library_ordinal - 1) as usize,
                            )) =>
                {
                    0
                }
                None => return Err(Error::NoExec),
            };
            let value = address
                .checked_add(binding.addend as i128)
                .ok_or(Error::Overflow)?;
            if !(0..=usize::MAX as i128).contains(&value) {
                return Err(Error::Libbad);
            }
            let target = self.runtime_address(binding.virtual_address)?;
            address_space.write(target, &(value as u64).to_le_bytes())?;
        }
        Ok(())
    }

    fn runtime_address(&self, virtual_address: u64) -> Result<usize, Error> {
        usize::try_from(self.load_bias + virtual_address as i128).map_err(|_| Error::Overflow)
    }

    fn load_reserved<F: FileSource, A: AddressSpace>(
        parsed: ParsedMachO<'_, F>,
        address_space: &mut A,
        mapping_base: usize,
        load_bias: i128,
    ) -> Result<Self, Error> {
        address_space.zero(mapping_base, parsed.mapping_length)?;
        for segment in &parsed.segments {
            if !segment.mapped || segment.file_size == 0 {
                continue;
            }
            let target = usize::try_from(load_bias + segment.virtual_address as i128)
                .map_err(|_| Error::Overflow)?;
            copy_file_range(
                parsed.source,
                address_space,
                segment.file_offset,
                target,
                segment.file_size,
            )?;
        }
        let mut exports = Vec::new();
        exports
            .try_reserve_exact(parsed.exports.len())
            .map_err(|_| Error::NoMem)?;
        for export in parsed.exports {
            let address = match export.address {
                ExportAddress::Relative(virtual_address) => {
                    usize::try_from(load_bias + virtual_address as i128)
                        .map_err(|_| Error::Overflow)?
                }
                ExportAddress::Absolute(address) => {
                    usize::try_from(address).map_err(|_| Error::Overflow)?
                }
            };
            exports.push(LoadedExport {
                name: export.name,
                address,
                weak: export.weak,
            });
        }
        let entry = parsed
            .entry_virtual_address
            .map(|virtual_address| {
                usize::try_from(load_bias + virtual_address as i128).map_err(|_| Error::Overflow)
            })
            .transpose()?;
        let pending = !parsed.bindings.is_empty()
            || (load_bias != 0 && !parsed.rebases.is_empty());
        Ok(Self {
            kind: parsed.kind,
            mapping_base,
            mapping_length: parsed.mapping_length,
            load_bias,
            entry,
            segments: parsed.segments,
            bindings: parsed.bindings,
            rebases: parsed.rebases,
            exports,
            needed: parsed.needed,
            needed_weak: parsed.needed_weak,
            rpaths: parsed.rpaths,
            state: if pending {
                ObjectState::RelocationPending
            } else {
                ObjectState::ReadyToSeal
            },
        })
    }
}

impl<'a, F: FileSource> ParsedMachO<'a, F> {
    fn parse(source: &'a F, kind: ObjectKind) -> Result<Self, Error> {
        if source.len() < MACH_HEADER_64_SIZE as u64 {
            return Err(Error::NoExec);
        }
        let mut header = [0u8; MACH_HEADER_64_SIZE];
        read_exact(source, 0, &mut header)?;
        if header[..4] != MH_MAGIC_64_LITTLE_ENDIAN {
            return Err(Error::OpNotSupp);
        }
        let cpu_type = endian::read_u32(&header, 4).ok_or(Error::NoExec)?;
        if cpu_type != NATIVE_CPU_TYPE {
            return Err(Error::OpNotSupp);
        }
        let file_type = endian::read_u32(&header, 12).ok_or(Error::NoExec)?;
        match (kind, file_type) {
            (ObjectKind::Executable, MH_EXECUTE)
            | (ObjectKind::SharedObject, MH_DYLIB | MH_BUNDLE) => {}
            _ => return Err(Error::NoExec),
        }
        let command_count = endian::read_u32(&header, 16).ok_or(Error::NoExec)? as usize;
        let command_bytes = endian::read_u32(&header, 20).ok_or(Error::NoExec)? as usize;
        let flags = endian::read_u32(&header, 24).ok_or(Error::NoExec)?;
        if command_count == 0 || command_count > MAX_LOAD_COMMANDS {
            return Err(Error::NoExec);
        }
        if command_bytes > MAX_LOAD_COMMAND_BYTES {
            return Err(Error::Fbig);
        }
        if MACH_HEADER_64_SIZE as u64 + command_bytes as u64 > source.len() {
            return Err(Error::NoExec);
        }
        let mut commands = Vec::new();
        commands
            .try_reserve_exact(command_bytes)
            .map_err(|_| Error::NoMem)?;
        commands.resize(command_bytes, 0);
        read_exact(source, MACH_HEADER_64_SIZE as u64, &mut commands)?;

        let mut segments = Vec::new();
        let mut needed = Vec::new();
        let mut needed_weak = Vec::new();
        let mut rpaths = Vec::new();
        let mut entry_offset = None;
        let mut saw_unix_thread = false;
        let mut symbol_table = None;
        let mut dyld_info = None;
        let mut export_trie = None;
        let mut chained_fixups = false;
        let mut cursor = 0usize;
        for _ in 0..command_count {
            if cursor.checked_add(LOAD_COMMAND_SIZE).ok_or(Error::Overflow)? > commands.len() {
                return Err(Error::NoExec);
            }
            let command_kind = endian::read_u32(&commands, cursor).ok_or(Error::NoExec)?;
            let command_size = endian::read_u32(&commands, cursor + 4).ok_or(Error::NoExec)? as usize;
            if command_size < LOAD_COMMAND_SIZE
                || command_size % 8 != 0
                || cursor.checked_add(command_size).ok_or(Error::Overflow)? > commands.len()
            {
                return Err(Error::NoExec);
            }
            let command = &commands[cursor..cursor + command_size];
            match command_kind {
                LC_SEGMENT_64 => {
                    if command_size < SEGMENT_64_SIZE || segments.len() >= MAX_SEGMENTS {
                        return Err(Error::NoExec);
                    }
                    let section_count = endian::read_u32(command, 64).ok_or(Error::NoExec)? as usize;
                    let section_bytes = section_count
                        .checked_mul(SECTION_64_SIZE)
                        .ok_or(Error::Overflow)?;
                    if SEGMENT_64_SIZE
                        .checked_add(section_bytes)
                        .ok_or(Error::Overflow)?
                        > command_size
                    {
                        return Err(Error::NoExec);
                    }
                    for section_index in 0..section_count {
                        let section_offset = SEGMENT_64_SIZE
                            .checked_add(
                                section_index
                                    .checked_mul(SECTION_64_SIZE)
                                    .ok_or(Error::Overflow)?,
                            )
                            .ok_or(Error::Overflow)?;
                        let section = command
                            .get(section_offset..section_offset + SECTION_64_SIZE)
                            .ok_or(Error::NoExec)?;
                        let section_type =
                            endian::read_u32(section, 64).ok_or(Error::NoExec)? & 0xff;
                        if matches!(
                            section_type,
                            S_MOD_INIT_FUNC_POINTERS | S_MOD_TERM_FUNC_POINTERS
                        ) || fixed_name_eq(&section[..16], b"__mod_init_func")
                            || fixed_name_eq(&section[..16], b"__mod_term_func")
                        {
                            return Err(Error::OpNotSupp);
                        }
                    }
                    let segment_name = &command[8..24];
                    let page_zero = fixed_name_eq(segment_name, b"__PAGEZERO");
                    let virtual_address = endian::read_u64(command, 24).ok_or(Error::NoExec)?;
                    let virtual_size = endian::read_u64(command, 32).ok_or(Error::NoExec)?;
                    let file_offset = endian::read_u64(command, 40).ok_or(Error::NoExec)?;
                    let file_size = endian::read_u64(command, 48).ok_or(Error::NoExec)?;
                    let max_protection = endian::read_u32(command, 56).ok_or(Error::NoExec)?;
                    let initial_protection = endian::read_u32(command, 60).ok_or(Error::NoExec)?;
                    if file_size > virtual_size
                        || virtual_address.checked_add(virtual_size).is_none()
                        || file_offset.checked_add(file_size).is_none()
                        || file_offset + file_size > source.len()
                        || initial_protection & !(VM_PROT_READ | VM_PROT_WRITE | VM_PROT_EXECUTE) != 0
                        || max_protection & !(VM_PROT_READ | VM_PROT_WRITE | VM_PROT_EXECUTE) != 0
                        || initial_protection & !max_protection != 0
                        || (file_size != 0
                            && virtual_address % PAGE_SIZE as u64
                                != file_offset % PAGE_SIZE as u64)
                    {
                        return Err(Error::NoExec);
                    }
                    let mapped = !page_zero && virtual_size != 0;
                    segments.try_reserve(1).map_err(|_| Error::NoMem)?;
                    segments.push(Segment {
                        virtual_address,
                        virtual_size,
                        file_offset,
                        file_size,
                        initial_protection,
                        maximum_protection: max_protection,
                        mapped,
                        linkedit: fixed_name_eq(segment_name, b"__LINKEDIT"),
                    });
                }
                LC_MAIN => {
                    if command_size < 24 || entry_offset.is_some() {
                        return Err(Error::NoExec);
                    }
                    entry_offset = Some(endian::read_u64(command, 8).ok_or(Error::NoExec)?);
                }
                LC_UNIXTHREAD => saw_unix_thread = true,
                LC_ROUTINES | LC_ROUTINES_64 => return Err(Error::OpNotSupp),
                LC_SYMTAB => {
                    if command_size < 24 || symbol_table.is_some() {
                        return Err(Error::NoExec);
                    }
                    symbol_table = Some(SymbolTable {
                        symbol_offset: endian::read_u32(command, 8).ok_or(Error::NoExec)?,
                        symbol_count: endian::read_u32(command, 12).ok_or(Error::NoExec)?,
                        string_offset: endian::read_u32(command, 16).ok_or(Error::NoExec)?,
                        string_size: endian::read_u32(command, 20).ok_or(Error::NoExec)?,
                    });
                }
                LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                    if command_size < 48 || dyld_info.is_some() {
                        return Err(Error::NoExec);
                    }
                    dyld_info = Some(DyldInfo {
                        rebase: FileRange {
                            offset: endian::read_u32(command, 8).ok_or(Error::NoExec)?,
                            size: endian::read_u32(command, 12).ok_or(Error::NoExec)?,
                        },
                        bind: FileRange {
                            offset: endian::read_u32(command, 16).ok_or(Error::NoExec)?,
                            size: endian::read_u32(command, 20).ok_or(Error::NoExec)?,
                        },
                        weak_bind: FileRange {
                            offset: endian::read_u32(command, 24).ok_or(Error::NoExec)?,
                            size: endian::read_u32(command, 28).ok_or(Error::NoExec)?,
                        },
                        lazy_bind: FileRange {
                            offset: endian::read_u32(command, 32).ok_or(Error::NoExec)?,
                            size: endian::read_u32(command, 36).ok_or(Error::NoExec)?,
                        },
                        exports: FileRange {
                            offset: endian::read_u32(command, 40).ok_or(Error::NoExec)?,
                            size: endian::read_u32(command, 44).ok_or(Error::NoExec)?,
                        },
                    });
                }
                LC_DYLD_EXPORTS_TRIE => {
                    if command_size < 16 || export_trie.is_some() {
                        return Err(Error::NoExec);
                    }
                    export_trie = Some(FileRange {
                        offset: endian::read_u32(command, 8).ok_or(Error::NoExec)?,
                        size: endian::read_u32(command, 12).ok_or(Error::NoExec)?,
                    });
                }
                LC_DYLD_CHAINED_FIXUPS => chained_fixups = true,
                LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB => {
                    if command_size < 24 {
                        return Err(Error::NoExec);
                    }
                    let name_offset = endian::read_u32(command, 8).ok_or(Error::NoExec)? as usize;
                    if name_offset < 24 {
                        return Err(Error::NoExec);
                    }
                    let name = command_string(command, name_offset)?;
                    if name.is_empty() {
                        return Err(Error::NoExec);
                    }
                    needed.try_reserve(1).map_err(|_| Error::NoMem)?;
                    needed.push(name);
                    needed_weak
                        .try_reserve(1)
                        .map_err(|_| Error::NoMem)?;
                    needed_weak.push(command_kind == LC_LOAD_WEAK_DYLIB);
                }
                LC_REEXPORT_DYLIB | LC_LOAD_UPWARD_DYLIB => return Err(Error::OpNotSupp),
                LC_RPATH => {
                    if command_size < 12 {
                        return Err(Error::NoExec);
                    }
                    let path_offset = endian::read_u32(command, 8).ok_or(Error::NoExec)? as usize;
                    if path_offset < 12 {
                        return Err(Error::NoExec);
                    }
                    let path = command_string(command, path_offset)?;
                    if path.is_empty() {
                        return Err(Error::NoExec);
                    }
                    rpaths.try_reserve(1).map_err(|_| Error::NoMem)?;
                    rpaths.push(path);
                }
                LC_LOAD_DYLINKER | LC_ID_DYLIB => {}
                _ if command_kind & LC_REQ_DYLD != 0 => return Err(Error::OpNotSupp),
                _ => {}
            }
            cursor += command_size;
        }
        if cursor != commands.len() || chained_fixups {
            return if chained_fixups {
                Err(Error::OpNotSupp)
            } else {
                Err(Error::NoExec)
            };
        }
        if let Some(info) = dyld_info {
            for range in [info.rebase, info.bind, info.weak_bind, info.lazy_bind, info.exports] {
                validate_linkedit_range(&segments, range)?;
            }
        }
        if let Some(range) = export_trie {
            validate_linkedit_range(&segments, range)?;
        }
        if let Some(table) = symbol_table {
            let symbol_size = table
                .symbol_count
                .checked_mul(16)
                .ok_or(Error::Overflow)?;
            validate_linkedit_range(
                &segments,
                FileRange {
                    offset: table.symbol_offset,
                    size: symbol_size,
                },
            )?;
            validate_linkedit_range(
                &segments,
                FileRange {
                    offset: table.string_offset,
                    size: table.string_size,
                },
            )?;
        }
        let mut minimum = usize::MAX;
        let mut maximum = 0usize;
        let mut ranges = Vec::new();
        ranges
            .try_reserve_exact(segments.len())
            .map_err(|_| Error::NoMem)?;
        for segment in &segments {
            if !segment.mapped {
                continue;
            }
            let start = usize::try_from(segment.virtual_address).map_err(|_| Error::Overflow)?;
            let end = usize::try_from(
                segment
                    .virtual_address
                    .checked_add(segment.virtual_size)
                    .ok_or(Error::Overflow)?,
            )
            .map_err(|_| Error::Overflow)?;
            let page_start = start & !(PAGE_SIZE - 1);
            let page_end = align_up(end, PAGE_SIZE).ok_or(Error::Overflow)?;
            minimum = minimum.min(page_start);
            maximum = maximum.max(page_end);
            ranges.push((segment.virtual_address, segment.virtual_address + segment.virtual_size));
        }
        if minimum == usize::MAX || maximum <= minimum {
            return Err(Error::NoExec);
        }
        ranges.sort_unstable_by_key(|range| range.0);
        if ranges.windows(2).any(|pair| pair[1].0 < pair[0].1) {
            return Err(Error::NoExec);
        }
        let mapping_length = maximum - minimum;
        if mapping_length > MAX_IMAGE_SIZE {
            return Err(Error::Fbig);
        }

        let entry_virtual_address = if kind == ObjectKind::Executable {
            let entry_offset = match entry_offset {
                Some(offset) => offset,
                None if saw_unix_thread => return Err(Error::OpNotSupp),
                None => return Err(Error::NoExec),
            };
            let Some(segment) = segments.iter().find(|segment| {
                segment.mapped
                    && entry_offset >= segment.file_offset
                    && entry_offset < segment.file_offset + segment.file_size
                    && segment.initial_protection & VM_PROT_EXECUTE != 0
            }) else {
                return Err(Error::NoExec);
            };
            Some(
                segment
                    .virtual_address
                    .checked_add(entry_offset - segment.file_offset)
                    .ok_or(Error::Overflow)?,
            )
        } else {
            None
        };
        let minimum_vm_address = minimum as u64;
        let mut bindings = Vec::new();
        let mut rebases = Vec::new();
        if let Some(info) = dyld_info {
            let stream = read_file_range(source, info.rebase)?;
            parse_rebase_stream(&stream, &segments, &mut rebases)?;
            let stream = read_file_range(source, info.bind)?;
            parse_bind_stream(&stream, &segments, &needed, false, false, &mut bindings)?;
            let stream = read_file_range(source, info.weak_bind)?;
            parse_bind_stream(&stream, &segments, &needed, true, false, &mut bindings)?;
            let stream = read_file_range(source, info.lazy_bind)?;
            parse_bind_stream(&stream, &segments, &needed, false, true, &mut bindings)?;
        }
        let export_range = export_trie.or_else(|| dyld_info.map(|info| info.exports));
        let mut exports = Vec::new();
        let has_export_trie = export_range.is_some_and(|range| range.size != 0);
        if let Some(range) = export_range.filter(|range| range.size != 0) {
            let trie = read_file_range(source, range)?;
            parse_export_trie(&trie, minimum_vm_address, &segments, &mut exports)?;
        }
        if !has_export_trie {
            if let Some(table) = symbol_table {
                parse_symbol_table(source, table, &segments, &mut exports)?;
            }
        }
        if kind == ObjectKind::Executable && entry_virtual_address.is_none() {
            return Err(Error::NoExec);
        }
        Ok(Self {
            source,
            kind,
            minimum_page: minimum,
            mapping_length,
            fixed_address: kind == ObjectKind::Executable && flags & MH_PIE == 0,
            segments,
            entry_virtual_address,
            bindings,
            rebases,
            exports,
            needed,
            needed_weak,
            rpaths,
        })
    }
}

pub(super) fn load<F: FileSource, A: AddressSpace>(
    source: &F,
    address_space: &mut A,
    kind: ObjectKind,
    preferred_address: Option<usize>,
) -> Result<super::dl::LoadedObjectState, Error> {
    let object = MachOLoadedObject::load(source, address_space, kind, preferred_address)?;
    Ok(super::dl::LoadedObjectState::MachO(object))
}

fn fixed_name_eq(field: &[u8], name: &[u8]) -> bool {
    field.get(..name.len()) == Some(name)
        && field
            .get(name.len())
            .is_some_and(|byte| *byte == 0 || name.len() == field.len())
}

fn command_string(command: &[u8], offset: usize) -> Result<Vec<u8>, Error> {
    let bytes = command.get(offset..).ok_or(Error::NoExec)?;
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::NoExec)?;
    if end > MAX_SYMBOL_NAME_SIZE {
        return Err(Error::Fbig);
    }
    let mut string = Vec::new();
    string.try_reserve_exact(end).map_err(|_| Error::NoMem)?;
    string.extend_from_slice(&bytes[..end]);
    Ok(string)
}

fn read_file_range<F: FileSource>(source: &F, range: FileRange) -> Result<Vec<u8>, Error> {
    if range.size as usize > MAX_LINKEDIT_DATA_SIZE {
        return Err(Error::Fbig);
    }
    if range.offset as u64 + range.size as u64 > source.len() {
        return Err(Error::NoExec);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(range.size as usize)
        .map_err(|_| Error::NoMem)?;
    bytes.resize(range.size as usize, 0);
    read_exact(source, range.offset as u64, &mut bytes)?;
    Ok(bytes)
}

fn validate_linkedit_range(segments: &[Segment], range: FileRange) -> Result<(), Error> {
    if range.size == 0 {
        return Ok(());
    }
    let start = range.offset as u64;
    let end = start.checked_add(range.size as u64).ok_or(Error::Overflow)?;
    if segments.iter().any(|segment| {
        segment.linkedit
            && start >= segment.file_offset
            && end <= segment.file_offset.saturating_add(segment.file_size)
    }) {
        Ok(())
    } else {
        Err(Error::NoExec)
    }
}

fn normalized_symbol(name: &[u8]) -> Result<Vec<u8>, Error> {
    let name = if name.first() == Some(&b'_') {
        &name[1..]
    } else {
        name
    };
    if name.is_empty() || name.len() > MAX_SYMBOL_NAME_SIZE {
        return Err(Error::NoExec);
    }
    let mut output = Vec::new();
    output.try_reserve_exact(name.len()).map_err(|_| Error::NoMem)?;
    output.extend_from_slice(name);
    Ok(output)
}

fn add_export(
    exports: &mut Vec<ParsedExport>,
    name: &[u8],
    address: ExportAddress,
    weak: bool,
) -> Result<(), Error> {
    if exports.len() >= MAX_SYMBOLS {
        return Err(Error::Fbig);
    }
    let name = normalized_symbol(name)?;
    if let Some(existing) = exports.iter_mut().find(|export| export.name == name) {
        if existing.weak && !weak {
            existing.address = address;
            existing.weak = false;
        }
        return Ok(());
    }
    exports.try_reserve(1).map_err(|_| Error::NoMem)?;
    exports.push(ParsedExport {
        name,
        address,
        weak,
    });
    Ok(())
}

fn parse_symbol_table<F: FileSource>(
    source: &F,
    table: SymbolTable,
    segments: &[Segment],
    exports: &mut Vec<ParsedExport>,
) -> Result<(), Error> {
    let symbol_bytes = (table.symbol_count as u64)
        .checked_mul(16)
        .ok_or(Error::Overflow)?;
    let symbol_end = table.symbol_offset as u64 + symbol_bytes;
    let string_end = table.string_offset as u64 + table.string_size as u64;
    if table.symbol_count as usize > MAX_SYMBOLS
        || symbol_end > source.len()
        || string_end > source.len()
        || table.string_size as usize > MAX_LINKEDIT_DATA_SIZE
    {
        return Err(Error::NoExec);
    }
    let mut strings = Vec::new();
    strings
        .try_reserve_exact(table.string_size as usize)
        .map_err(|_| Error::NoMem)?;
    strings.resize(table.string_size as usize, 0);
    read_exact(source, table.string_offset as u64, &mut strings)?;
    let mut symbol = [0u8; 16];
    for index in 0..table.symbol_count as usize {
        let offset = table.symbol_offset as u64
            + (index as u64)
                .checked_mul(16)
                .ok_or(Error::Overflow)?;
        read_exact(source, offset, &mut symbol)?;
        let string_index = endian::read_u32(&symbol, 0).ok_or(Error::NoExec)? as usize;
        let symbol_type = symbol[4];
        if symbol_type & N_STAB != 0
            || symbol_type & N_EXT == 0
            || symbol_type & N_PEXT != 0
        {
            continue;
        }
        let kind = symbol_type & N_TYPE;
        if kind == N_UNDF {
            continue;
        }
        let end = strings
            .get(string_index..)
            .ok_or(Error::NoExec)?
            .iter()
            .position(|byte| *byte == 0)
            .and_then(|length| string_index.checked_add(length))
            .ok_or(Error::NoExec)?;
        let name = &strings[string_index..end];
        let value = endian::read_u64(&symbol, 8).ok_or(Error::NoExec)?;
        let (address, weak) = match kind {
            N_ABS => (ExportAddress::Absolute(value), false),
            N_SECT => {
                if !contains_vm_address(segments, value, 1) {
                    return Err(Error::NoExec);
                }
                let description = endian::read_u16(&symbol, 6).ok_or(Error::NoExec)?;
                (
                    ExportAddress::Relative(value),
                    description & N_WEAK_DEF != 0,
                )
            }
            _ => continue,
        };
        add_export(exports, name, address, weak)?;
    }
    Ok(())
}

fn parse_export_trie(
    trie: &[u8],
    image_base: u64,
    segments: &[Segment],
    exports: &mut Vec<ParsedExport>,
) -> Result<(), Error> {
    if trie.is_empty() {
        return Ok(());
    }
    let mut pending = Vec::new();
    pending.try_reserve(1).map_err(|_| Error::NoMem)?;
    pending.push((0usize, Vec::new()));
    let mut visited = 0usize;
    let mut total_name_bytes = 0usize;
    while let Some((node_offset, prefix)) = pending.pop() {
        visited = visited.checked_add(1).ok_or(Error::Overflow)?;
        if visited > MAX_SYMBOLS || node_offset >= trie.len() {
            return Err(Error::Fbig);
        }
        let mut cursor = node_offset;
        let terminal_size = usize::try_from(read_uleb(trie, &mut cursor)?)
            .map_err(|_| Error::Overflow)?;
        let terminal_end = cursor
            .checked_add(terminal_size)
            .ok_or(Error::Overflow)?;
        if terminal_end > trie.len() {
            return Err(Error::NoExec);
        }
        if terminal_size != 0 {
            let terminal = &trie[cursor..terminal_end];
            let mut terminal_cursor = 0usize;
            let flags = read_uleb(terminal, &mut terminal_cursor)?;
            if flags & EXPORT_SYMBOL_FLAGS_REEXPORT != 0
                || flags & EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER != 0
            {
                return Err(Error::OpNotSupp);
            }
            let address = read_uleb(terminal, &mut terminal_cursor)?;
            let kind = flags & EXPORT_SYMBOL_FLAGS_KIND_MASK;
            if kind == EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL {
                return Err(Error::OpNotSupp);
            }
            let address = match kind {
                EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE => ExportAddress::Absolute(address),
                0 => {
                    let virtual_address = image_base.checked_add(address).ok_or(Error::Overflow)?;
                    if !contains_vm_address(segments, virtual_address, 1) {
                        return Err(Error::NoExec);
                    }
                    ExportAddress::Relative(virtual_address)
                }
                _ => return Err(Error::OpNotSupp),
            };
            add_export(
                exports,
                &prefix,
                address,
                flags & EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION != 0,
            )?;
        }
        cursor = terminal_end;
        let child_count = *trie.get(cursor).ok_or(Error::NoExec)? as usize;
        cursor += 1;
        for _ in 0..child_count {
            let edge_start = cursor;
            let edge_end = trie
                .get(cursor..)
                .ok_or(Error::NoExec)?
                .iter()
                .position(|byte| *byte == 0)
                .and_then(|length| cursor.checked_add(length))
                .ok_or(Error::NoExec)?;
            let edge = &trie[edge_start..edge_end];
            cursor = edge_end.checked_add(1).ok_or(Error::Overflow)?;
            let child_offset = usize::try_from(read_uleb(trie, &mut cursor)?)
                .map_err(|_| Error::Overflow)?;
            let name_length = prefix.len().checked_add(edge.len()).ok_or(Error::Overflow)?;
            if name_length > MAX_SYMBOL_NAME_SIZE {
                return Err(Error::Fbig);
            }
            total_name_bytes = total_name_bytes
                .checked_add(name_length)
                .ok_or(Error::Overflow)?;
            if total_name_bytes > MAX_LINKEDIT_DATA_SIZE {
                return Err(Error::Fbig);
            }
            let mut name = Vec::new();
            name.try_reserve_exact(name_length)
                .map_err(|_| Error::NoMem)?;
            name.extend_from_slice(&prefix);
            name.extend_from_slice(edge);
            pending.try_reserve(1).map_err(|_| Error::NoMem)?;
            pending.push((child_offset, name));
        }
    }
    Ok(())
}

fn parse_rebase_stream(
    stream: &[u8],
    segments: &[Segment],
    output: &mut Vec<u64>,
) -> Result<(), Error> {
    let mut cursor = 0usize;
    let mut segment_index = None;
    let mut segment_offset = 0u64;
    let mut rebase_type = 0u8;
    let mut terminated = stream.is_empty();
    while cursor < stream.len() {
        let opcode = stream[cursor] & 0xf0;
        let immediate = stream[cursor] & 0x0f;
        cursor += 1;
        match opcode {
            0x00 => {
                terminated = true;
                break;
            }
            0x10 => rebase_type = immediate,
            0x20 => {
                let index = immediate as usize;
                let offset = read_uleb(stream, &mut cursor)?;
                validate_segment_offset(segments, index, offset, 0)?;
                segment_index = Some(index);
                segment_offset = offset;
            }
            0x30 => {
                segment_offset = segment_offset
                    .checked_add(read_uleb(stream, &mut cursor)?)
                    .ok_or(Error::Overflow)?;
            }
            0x40 => {
                segment_offset = segment_offset
                    .checked_add((immediate as u64).checked_mul(8).ok_or(Error::Overflow)?)
                    .ok_or(Error::Overflow)?;
            }
            0x50 => {
                for _ in 0..immediate {
                    append_rebase(segments, segment_index, segment_offset, rebase_type, output)?;
                    segment_offset = segment_offset.checked_add(8).ok_or(Error::Overflow)?;
                }
            }
            0x60 => {
                let count = read_uleb(stream, &mut cursor)?;
                if count > MAX_RELOCATIONS as u64 {
                    return Err(Error::Fbig);
                }
                for _ in 0..count {
                    append_rebase(segments, segment_index, segment_offset, rebase_type, output)?;
                    segment_offset = segment_offset.checked_add(8).ok_or(Error::Overflow)?;
                }
            }
            0x70 => {
                let skip = read_uleb(stream, &mut cursor)?;
                append_rebase(segments, segment_index, segment_offset, rebase_type, output)?;
                segment_offset = segment_offset
                    .checked_add(8)
                    .and_then(|offset| offset.checked_add(skip))
                    .ok_or(Error::Overflow)?;
            }
            0x80 => {
                let count = read_uleb(stream, &mut cursor)?;
                let skip = read_uleb(stream, &mut cursor)?;
                if count > MAX_RELOCATIONS as u64 {
                    return Err(Error::Fbig);
                }
                for _ in 0..count {
                    append_rebase(segments, segment_index, segment_offset, rebase_type, output)?;
                    segment_offset = segment_offset
                        .checked_add(8)
                        .and_then(|offset| offset.checked_add(skip))
                        .ok_or(Error::Overflow)?;
                }
            }
            _ => return Err(Error::OpNotSupp),
        }
    }
    if !terminated {
        return Err(Error::NoExec);
    }
    Ok(())
}

fn append_rebase(
    segments: &[Segment],
    segment_index: Option<usize>,
    segment_offset: u64,
    rebase_type: u8,
    output: &mut Vec<u64>,
) -> Result<(), Error> {
    if rebase_type != BIND_TYPE_POINTER {
        return Err(Error::OpNotSupp);
    }
    let index = segment_index.ok_or(Error::NoExec)?;
    validate_segment_offset(segments, index, segment_offset, 8)?;
    if output.len() >= MAX_RELOCATIONS {
        return Err(Error::Fbig);
    }
    let address = segments[index]
        .virtual_address
        .checked_add(segment_offset)
        .ok_or(Error::Overflow)?;
    output.try_reserve(1).map_err(|_| Error::NoMem)?;
    output.push(address);
    Ok(())
}

fn parse_bind_stream(
    stream: &[u8],
    segments: &[Segment],
    needed: &[Vec<u8>],
    weak_stream: bool,
    lazy_stream: bool,
    output: &mut Vec<Binding>,
) -> Result<(), Error> {
    let mut cursor = 0usize;
    let mut segment_index = None;
    let mut segment_offset = 0u64;
    let mut symbol_name = None;
    let mut library_ordinal = 0i32;
    let mut addend = 0i64;
    let mut bind_type = 0u8;
    let mut weak_import = false;
    let mut terminated = stream.is_empty();
    while cursor < stream.len() {
        let opcode = stream[cursor] & 0xf0;
        let immediate = stream[cursor] & 0x0f;
        cursor += 1;
        match opcode {
            0x00 => {
                if lazy_stream && cursor < stream.len() {
                    segment_index = None;
                    segment_offset = 0;
                    symbol_name = None;
                    library_ordinal = 0;
                    addend = 0;
                    bind_type = 0;
                    weak_import = false;
                    continue;
                }
                terminated = true;
                break;
            }
            0x10 => library_ordinal = immediate as i32,
            0x20 => {
                library_ordinal = i32::try_from(read_uleb(stream, &mut cursor)?)
                    .map_err(|_| Error::Overflow)?;
            }
            0x30 => {
                library_ordinal = match immediate {
                    0 => 0,
                    0x0d..=0x0f => immediate as i32 - 16,
                    _ => return Err(Error::OpNotSupp),
                };
            }
            0x40 => {
                let flags = immediate;
                let name = read_stream_string(stream, &mut cursor)?;
                symbol_name = Some(normalized_symbol(&name)?);
                weak_import = flags & BIND_SYMBOL_FLAGS_WEAK_IMPORT != 0;
            }
            0x50 => bind_type = immediate,
            0x60 => addend = read_sleb(stream, &mut cursor)?,
            0x70 => {
                let index = immediate as usize;
                let offset = read_uleb(stream, &mut cursor)?;
                validate_segment_offset(segments, index, offset, 0)?;
                segment_index = Some(index);
                segment_offset = offset;
            }
            0x80 => {
                segment_offset = segment_offset
                    .checked_add(read_uleb(stream, &mut cursor)?)
                    .ok_or(Error::Overflow)?;
            }
            0x90 => {
                append_binding(
                    segments,
                    segment_index,
                    segment_offset,
                    bind_type,
                    library_ordinal,
                    symbol_name.as_deref(),
                    addend,
                    weak_import || weak_stream || library_ordinal == -3,
                    needed,
                    output,
                )?;
                segment_offset = segment_offset.checked_add(8).ok_or(Error::Overflow)?;
            }
            0xa0 => {
                let skip = read_uleb(stream, &mut cursor)?;
                append_binding(
                    segments,
                    segment_index,
                    segment_offset,
                    bind_type,
                    library_ordinal,
                    symbol_name.as_deref(),
                    addend,
                    weak_import || weak_stream || library_ordinal == -3,
                    needed,
                    output,
                )?;
                segment_offset = segment_offset
                    .checked_add(8)
                    .and_then(|offset| offset.checked_add(skip))
                    .ok_or(Error::Overflow)?;
            }
            0xb0 => {
                append_binding(
                    segments,
                    segment_index,
                    segment_offset,
                    bind_type,
                    library_ordinal,
                    symbol_name.as_deref(),
                    addend,
                    weak_import || weak_stream || library_ordinal == -3,
                    needed,
                    output,
                )?;
                let skip = (immediate as u64).checked_mul(8).ok_or(Error::Overflow)?;
                segment_offset = segment_offset
                    .checked_add(8)
                    .and_then(|offset| offset.checked_add(skip))
                    .ok_or(Error::Overflow)?;
            }
            0xc0 => {
                let count = read_uleb(stream, &mut cursor)?;
                let skip = read_uleb(stream, &mut cursor)?;
                if count > MAX_RELOCATIONS as u64 {
                    return Err(Error::Fbig);
                }
                for _ in 0..count {
                    append_binding(
                        segments,
                        segment_index,
                        segment_offset,
                        bind_type,
                        library_ordinal,
                        symbol_name.as_deref(),
                        addend,
                        weak_import || weak_stream || library_ordinal == -3,
                        needed,
                        output,
                    )?;
                    segment_offset = segment_offset
                        .checked_add(8)
                        .and_then(|offset| offset.checked_add(skip))
                        .ok_or(Error::Overflow)?;
                }
            }
            0xd0 => return Err(Error::OpNotSupp),
            _ => return Err(Error::OpNotSupp),
        }
    }
    if terminated {
        Ok(())
    } else {
        Err(Error::NoExec)
    }
}

fn append_binding(
    segments: &[Segment],
    segment_index: Option<usize>,
    segment_offset: u64,
    bind_type: u8,
    library_ordinal: i32,
    symbol_name: Option<&[u8]>,
    addend: i64,
    weak: bool,
    needed: &[Vec<u8>],
    output: &mut Vec<Binding>,
) -> Result<(), Error> {
    if bind_type != BIND_TYPE_POINTER {
        return Err(Error::OpNotSupp);
    }
    let index = segment_index.ok_or(Error::NoExec)?;
    validate_segment_offset(segments, index, segment_offset, 8)?;
    if library_ordinal > 0 && library_ordinal as usize > needed.len() {
        return Err(Error::NoExec);
    }
    if library_ordinal == -1 {
        return Err(Error::OpNotSupp);
    }
    if library_ordinal < -3 {
        return Err(Error::OpNotSupp);
    }
    let name = symbol_name.ok_or(Error::NoExec)?;
    if output.len() >= MAX_RELOCATIONS {
        return Err(Error::Fbig);
    }
    let virtual_address = segments[index]
        .virtual_address
        .checked_add(segment_offset)
        .ok_or(Error::Overflow)?;
    let mut name_copy = Vec::new();
    name_copy
        .try_reserve_exact(name.len())
        .map_err(|_| Error::NoMem)?;
    name_copy.extend_from_slice(name);
    output.try_reserve(1).map_err(|_| Error::NoMem)?;
    output.push(Binding {
        virtual_address,
        name: name_copy,
        library_ordinal,
        addend,
        weak,
    });
    Ok(())
}

fn validate_segment_offset(
    segments: &[Segment],
    index: usize,
    offset: u64,
    width: u64,
) -> Result<(), Error> {
    let segment = segments.get(index).ok_or(Error::NoExec)?;
    if !segment.mapped
        || offset.checked_add(width).ok_or(Error::Overflow)? > segment.virtual_size
    {
        return Err(Error::NoExec);
    }
    if segment.maximum_protection & VM_PROT_WRITE == 0 {
        return Err(Error::Libbad);
    }
    Ok(())
}

fn contains_vm_address(segments: &[Segment], address: u64, width: u64) -> bool {
    let Some(end) = address.checked_add(width) else {
        return false;
    };
    segments.iter().any(|segment| {
        segment.mapped
            && address >= segment.virtual_address
            && end <= segment.virtual_address + segment.virtual_size
    })
}

fn read_stream_string(stream: &[u8], cursor: &mut usize) -> Result<Vec<u8>, Error> {
    let start = *cursor;
    let end = stream
        .get(start..)
        .ok_or(Error::NoExec)?
        .iter()
        .position(|byte| *byte == 0)
        .and_then(|length| start.checked_add(length))
        .ok_or(Error::NoExec)?;
    if end - start > MAX_SYMBOL_NAME_SIZE {
        return Err(Error::Fbig);
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(end - start)
        .map_err(|_| Error::NoMem)?;
    output.extend_from_slice(&stream[start..end]);
    *cursor = end.checked_add(1).ok_or(Error::Overflow)?;
    Ok(output)
}

fn read_uleb(bytes: &[u8], cursor: &mut usize) -> Result<u64, Error> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*cursor).ok_or(Error::NoExec)?;
        *cursor = cursor.checked_add(1).ok_or(Error::Overflow)?;
        let payload = (byte & 0x7f) as u64;
        if shift >= 64 || (shift == 63 && payload > 1) {
            return Err(Error::Overflow);
        }
        value |= payload << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift = shift.checked_add(7).ok_or(Error::Overflow)?;
        if shift >= 70 {
            return Err(Error::Overflow);
        }
    }
}

fn read_sleb(bytes: &[u8], cursor: &mut usize) -> Result<i64, Error> {
    let mut value = 0i128;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*cursor).ok_or(Error::NoExec)?;
        *cursor = cursor.checked_add(1).ok_or(Error::Overflow)?;
        let payload = (byte & 0x7f) as i128;
        if shift >= 70 {
            return Err(Error::Overflow);
        }
        value |= payload << shift;
        shift = shift.checked_add(7).ok_or(Error::Overflow)?;
        if byte & 0x80 == 0 {
            if byte & 0x40 != 0 {
                value |= !0i128 << shift;
            }
            return i64::try_from(value).map_err(|_| Error::Overflow);
        }
        if shift >= 70 {
            return Err(Error::Overflow);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/kernel/dl/mach_o.rs"]
mod tests;
