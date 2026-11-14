/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

//! Bounded OTP-27 BEAM format-0 loader and interpreter subset.
//!
//! This is not an OTP-compatible runtime: heap terms, module calls, receive,
//! processes, and most generic opcodes trap with `NoSys` when executed. The optional `beam-jit`
//! feature compiles the supported instruction subset to x86_64 code, with interpreter fallback.
//! Integers use checked `i64` arithmetic; BEAM bignums and floats are unsupported.

use alloc::vec::Vec;
#[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
use alloc::sync::Arc;

use crate::kernel::error::Error;
#[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
use crate::kernel::scheduler::sync::IrqSpinLock;

#[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
mod jit;
#[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
pub(crate) use jit::JitFunction;

const BEAM_FORMAT_NUMBER: u32 = 0;
const MAX_GENERIC_OPCODE: u32 = 183;
const MAX_BEAM_BYTES: usize = 16 * 1024 * 1024;
const MAX_CODE_BYTES: usize = 8 * 1024 * 1024;
const MAX_INSTRUCTIONS: usize = 1 << 16;
const MAX_TABLE_ENTRIES: usize = 1 << 16;
const MAX_REGISTERS: usize = 256;
const MAX_STACK_WORDS: usize = 256;
const MAX_EXECUTION_STEPS: usize = 1 << 20;

const TAG_UNSIGNED: u8 = 0;
const TAG_INTEGER: u8 = 1;
const TAG_ATOM: u8 = 2;
const TAG_X_REGISTER: u8 = 3;
const TAG_Y_REGISTER: u8 = 4;
const TAG_LABEL: u8 = 5;
const TAG_EXTENDED: u8 = 7;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Term {
    Integer(i64),
    Atom(u32),
    Nil,
}

impl Term {
    pub(crate) const fn as_integer(self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(value),
            Self::Atom(_) | Self::Nil => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operand {
    Unsigned(u64),
    Integer(i64),
    Atom(u32),
    X(usize),
    Y(usize),
    Label(usize),
    Nil,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Source {
    Term(Term),
    X(usize),
    Y(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Destination {
    X(usize),
    Y(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Bif1 {
    Negate,
    Absolute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Bif2 {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    BitAnd,
    BitOr,
    BitXor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Compare {
    Equal,
    ExactEqual,
    NotEqual,
    ExactNotEqual,
    LessThan,
    GreaterOrEqual,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Instruction {
    Noop,
    Label(usize),
    Unsupported,
    Move {
        source: Source,
        destination: Destination,
    },
    Bif1 {
        fail: Option<usize>,
        operation: Bif1,
        source: Source,
        destination: Destination,
    },
    Bif2 {
        fail: Option<usize>,
        operation: Bif2,
        left: Source,
        right: Source,
        destination: Destination,
    },
    Compare {
        fail: Option<usize>,
        operation: Compare,
        left: Source,
        right: Source,
    },
    IsInteger {
        fail: Option<usize>,
        source: Source,
    },
    Jump(usize),
    Allocate(usize),
    Deallocate,
    InitializeY(usize),
    Return,
}

#[derive(Clone, Copy)]
struct Import {
    module_atom: u32,
    function_atom: u32,
    arity: u32,
}

#[derive(Clone, Copy)]
struct Export {
    function_atom: u32,
    arity: u32,
    label: usize,
}

pub(crate) struct BeamModule {
    //@FIXME: INDEXTYPE(atoms); atom-ID-indexed dynamic table of byte strings; Vec access is O(1), keep Vec.
    atoms: Vec<Vec<u8>>,
    //@FIXME: INDEXTYPE(exports); ordered first-match name/arity scan is O(n); keep Vec unless a semantics-preserving hash side index is warranted.
    exports: Vec<Export>,
    //@FIXME: INDEXTYPE(instructions); dense program-counter-indexed bytecode array; Vec access is O(1), keep Vec.
    instructions: Vec<Instruction>,
    //@FIXME: INDEXTYPE(labels); label-ID-indexed table; Vec access is O(1), keep Vec.
    labels: Vec<Option<usize>>,
    #[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
    //@FIXME: INDEXTYPE(jit_cache); linear lookup is bounded to 16 entries; keep Vec.
    jit_cache: IrqSpinLock<Vec<CachedJitFunction>>,
}

#[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
struct CachedJitFunction {
    function_atom: u32,
    arity: u32,
    function: Arc<JitFunction>,
}

#[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
const MAX_CACHED_JIT_FUNCTIONS: usize = 16;

impl BeamModule {
    pub(crate) fn load(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 12 {
            return Err(Error::NoExec);
        }
        if bytes.len() > MAX_BEAM_BYTES {
            return Err(Error::TooBig);
        }
        let form = BeamForm::parse(bytes)?;
        let atom_chunk = form
            .chunk(*b"AtU8")
            .or_else(|| form.chunk(*b"Atom"))
            .ok_or(Error::NoExec)?;
        let atoms = parse_atoms(atom_chunk, form.chunk(*b"AtU8").is_some())?;
        let code_chunk = form.chunk(*b"Code").ok_or(Error::NoExec)?;
        let imports = form
            .chunk(*b"ImpT")
            .map(parse_imports)
            .transpose()?
            .unwrap_or_default();
        let (header, instructions, labels) = parse_code(code_chunk, &atoms, &imports)?;
        let exports = form
            .chunk(*b"ExpT")
            .map(|chunk| parse_exports(chunk, &atoms, header.label_count))
            .transpose()?
            .ok_or(Error::NoExec)?;
        if exports
            .iter()
            .any(|export| labels.get(export.label).and_then(|target| *target).is_none())
        {
            return Err(Error::Proto);
        }

        #[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
        let mut jit_cache = Vec::new();
        #[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
        jit_cache
            .try_reserve_exact(MAX_CACHED_JIT_FUNCTIONS)
            .map_err(|_| Error::NoMem)?;

        Ok(Self {
            atoms,
            exports,
            instructions,
            labels,
            #[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
            jit_cache: IrqSpinLock::new(jit_cache),
        })
    }

    pub(crate) fn atom_name(&self, term: Term) -> Option<&[u8]> {
        let Term::Atom(index) = term else {
            return None;
        };
        self.atoms.get(index as usize).map(Vec::as_slice)
    }

    pub(crate) fn run_export(&self, name: &[u8], arguments: &[Term]) -> Result<Term, Error> {
        #[cfg(feature = "beam-jit")]
        {
            return self.run_export_auto(name, arguments);
        }
        #[cfg(not(feature = "beam-jit"))]
        self.run_export_interpreted(name, arguments)
    }

    fn run_export_interpreted(&self, name: &[u8], arguments: &[Term]) -> Result<Term, Error> {
        let export = self.find_export(name, arguments.len())?;
        let mut registers = [Term::Nil; MAX_REGISTERS];
        registers[..arguments.len()].copy_from_slice(arguments);
        let mut stack = [Term::Nil; MAX_STACK_WORDS];
        let mut stack_length = 0usize;
        let mut pc = self.label_target(export.label)?;

        for _ in 0..MAX_EXECUTION_STEPS {
            let instruction = *self.instructions.get(pc).ok_or(Error::NoExec)?;
            pc = pc.checked_add(1).ok_or(Error::Overflow)?;
            match instruction {
                Instruction::Noop | Instruction::Label(_) => {}
                Instruction::Unsupported => return Err(Error::NoSys),
                Instruction::Move {
                    source,
                    destination,
                } => {
                    let value = read_source(source, &registers, &stack, stack_length)?;
                    write_destination(destination, value, &mut registers, &mut stack, stack_length)?;
                }
                Instruction::Bif1 {
                    fail,
                    operation,
                    source,
                    destination,
                } => {
                    let result = read_source(source, &registers, &stack, stack_length)
                        .and_then(|term| execute_bif1(operation, term));
                    match result {
                        Ok(value) => write_destination(
                            destination,
                            value,
                            &mut registers,
                            &mut stack,
                            stack_length,
                        )?,
                        Err(error) => match fail {
                            Some(label) => pc = self.label_target(label)?,
                            None => return Err(error),
                        },
                    }
                }
                Instruction::Bif2 {
                    fail,
                    operation,
                    left,
                    right,
                    destination,
                } => {
                    let result = read_source(left, &registers, &stack, stack_length)
                        .and_then(|left| read_source(right, &registers, &stack, stack_length).map(|right| (left, right)))
                        .and_then(|(left, right)| execute_bif2(operation, left, right));
                    match result {
                        Ok(value) => write_destination(
                            destination,
                            value,
                            &mut registers,
                            &mut stack,
                            stack_length,
                        )?,
                        Err(error) => match fail {
                            Some(label) => pc = self.label_target(label)?,
                            None => return Err(error),
                        },
                    }
                }
                Instruction::Compare {
                    fail,
                    operation,
                    left,
                    right,
                } => {
                    let left = read_source(left, &registers, &stack, stack_length)?;
                    let right = read_source(right, &registers, &stack, stack_length)?;
                    if !compare_terms(operation, left, right)? {
                        pc = match fail {
                            Some(label) => self.label_target(label)?,
                            None => return Err(Error::Inval),
                        };
                    }
                }
                Instruction::IsInteger { fail, source } => {
                    if !matches!(read_source(source, &registers, &stack, stack_length)?, Term::Integer(_)) {
                        pc = match fail {
                            Some(label) => self.label_target(label)?,
                            None => return Err(Error::Inval),
                        };
                    }
                }
                Instruction::Jump(label) => pc = self.label_target(label)?,
                Instruction::Allocate(words) => {
                    if words > stack.len() {
                        return Err(Error::NoMem);
                    }
                    stack[..words].fill(Term::Nil);
                    stack_length = words;
                }
                Instruction::Deallocate => {
                    stack[..stack_length].fill(Term::Nil);
                    stack_length = 0;
                }
                Instruction::InitializeY(index) => {
                    let slot = stack.get_mut(index).filter(|_| index < stack_length).ok_or(Error::NoExec)?;
                    *slot = Term::Nil;
                }
                Instruction::Return => return Ok(registers[0]),
            }
        }
        Err(Error::TimedOut)
    }

    fn find_export(&self, name: &[u8], arity: usize) -> Result<Export, Error> {
        let arity = u32::try_from(arity).map_err(|_| Error::Inval)?;
        self.exports
            .iter()
            .copied()
            .find(|export| {
                export.arity == arity
                    && self
                        .atoms
                        .get(export.function_atom as usize)
                        .is_some_and(|atom| atom.as_slice() == name)
            })
            .ok_or(Error::NoEnt)
    }

    fn label_target(&self, label: usize) -> Result<usize, Error> {
        self.labels
            .get(label)
            .copied()
            .flatten()
            .ok_or(Error::NoExec)
    }

    #[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
    pub(crate) fn compile_export(&self, name: &[u8], arity: usize) -> Result<JitFunction, Error> {
        let export = self.find_export(name, arity)?;
        let start = self.label_target(export.label)?;
        JitFunction::compile(&self.instructions, &self.labels, start, arity)
    }

    #[cfg(all(feature = "beam-jit", target_arch = "x86_64"))]
    fn get_or_compile_export(
        &self,
        name: &[u8],
        arity: usize,
    ) -> Result<Arc<JitFunction>, Error> {
        let export = self.find_export(name, arity)?;
        {
            let cache = self.jit_cache.lock();
            if let Some(cached) = cache.iter().find(|cached| {
                cached.function_atom == export.function_atom && cached.arity == export.arity
            }) {
                return Ok(Arc::clone(&cached.function));
            }
        }

        let start = self.label_target(export.label)?;
        let compiled = Arc::new(JitFunction::compile(
            &self.instructions,
            &self.labels,
            start,
            arity,
        )?);
        let mut cache = self.jit_cache.lock();
        if let Some(cached) = cache.iter().find(|cached| {
            cached.function_atom == export.function_atom && cached.arity == export.arity
        }) {
            return Ok(Arc::clone(&cached.function));
        }
        if cache.len() < MAX_CACHED_JIT_FUNCTIONS {
            cache.push(CachedJitFunction {
                function_atom: export.function_atom,
                arity: export.arity,
                function: Arc::clone(&compiled),
            });
        }
        Ok(compiled)
    }

    #[cfg(feature = "beam-jit")]
    pub(crate) fn run_export_auto(&self, name: &[u8], arguments: &[Term]) -> Result<Term, Error> {
        #[cfg(target_arch = "x86_64")]
        {
            match self.get_or_compile_export(name, arguments.len()) {
                Ok(compiled) => match compiled.run(arguments) {
                    Ok(result) => return Ok(result),
                    Err(Error::NoSys) => {}
                    Err(error) => return Err(error),
                },
                Err(Error::NoSys) | Err(Error::TooBig) | Err(Error::NoMem) => {}
                Err(error) => return Err(error),
            }
        }
        self.run_export_interpreted(name, arguments)
    }
}

#[derive(Clone, Copy)]
struct CodeHeader {
    label_count: usize,
}

struct BeamForm<'a> {
    bytes: &'a [u8],
    end: usize,
}

impl<'a> BeamForm<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.get(..4) != Some(&b"FOR1"[..]) || bytes.get(8..12) != Some(&b"BEAM"[..]) {
            return Err(Error::NoExec);
        }
        let form_length = read_be_u32(bytes, 4)? as usize;
        let end = 8usize.checked_add(form_length).ok_or(Error::Proto)?;
        if form_length < 4 || end != bytes.len() {
            return Err(Error::Proto);
        }
        Ok(Self { bytes, end })
    }

    fn chunk(&self, name: [u8; 4]) -> Option<&'a [u8]> {
        let mut cursor = 12usize;
        while cursor < self.end {
            let header_end = cursor.checked_add(8)?;
            if header_end > self.end {
                return None;
            }
            let chunk_name = self.bytes.get(cursor..cursor + 4)?;
            let chunk_length = read_be_u32(self.bytes, cursor + 4).ok()? as usize;
            let chunk_end = header_end.checked_add(chunk_length)?;
            let padded_length = chunk_length.checked_add(3)? & !3;
            let next = header_end.checked_add(padded_length)?;
            if chunk_end > self.end || next > self.end {
                return None;
            }
            if chunk_name == name.as_slice() {
                return self.bytes.get(header_end..chunk_end);
            }
            cursor = next;
        }
        None
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

#[derive(Clone, Copy)]
struct TaggedValue {
    tag: u8,
    value: u64,
    integer: i64,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.offset)
    }

    fn read_u8(&mut self) -> Result<u8, Error> {
        let value = *self.bytes.get(self.offset).ok_or(Error::Proto)?;
        self.offset = self.offset.checked_add(1).ok_or(Error::Proto)?;
        Ok(value)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], Error> {
        let end = self.offset.checked_add(length).ok_or(Error::Proto)?;
        let result = self.bytes.get(self.offset..end).ok_or(Error::Proto)?;
        self.offset = end;
        Ok(result)
    }

    fn read_tagged(&mut self) -> Result<TaggedValue, Error> {
        self.read_tagged_depth(0)
    }

    fn read_tagged_depth(&mut self, depth: usize) -> Result<TaggedValue, Error> {
        if depth > 4 {
            return Err(Error::Proto);
        }
        let length_code = self.read_u8()?;
        let tag = length_code & 0x07;
        let (value, payload_length) = if length_code & 0x08 == 0 {
            (u64::from(length_code >> 4), None)
        } else if length_code & 0x10 == 0 {
            let low = u64::from(self.read_u8()?);
            ((u64::from(length_code >> 5) << 8) | low, None)
        } else {
            let size_code = length_code >> 5;
            let size = if size_code < 7 {
                usize::from(size_code) + 2
            } else {
                let size_value = self.read_tagged_depth(depth + 1)?;
                if size_value.tag != TAG_UNSIGNED {
                    return Err(Error::Proto);
                }
                usize::try_from(size_value.value)
                    .map_err(|_| Error::TooBig)?
                    .checked_add(9)
                    .ok_or(Error::Proto)?
            };
            if size > 8 {
                return Err(Error::NoSys);
            }
            let data = self.read_bytes(size)?;
            let mut value = 0u64;
            for byte in data {
                value = (value << 8) | u64::from(*byte);
            }
            (value, Some(size))
        };
        let integer = if tag == TAG_INTEGER {
            if let Some(size) = payload_length {
                if size < 8 && value & (1u64 << (size * 8 - 1)) != 0 {
                    (value | (u64::MAX << (size * 8))) as i64
                } else {
                    value as i64
                }
            } else {
                i64::try_from(value).map_err(|_| Error::Overflow)?
            }
        } else {
            i64::try_from(value).unwrap_or(i64::MAX)
        };
        Ok(TaggedValue { tag, value, integer })
    }

    fn read_operand(&mut self) -> Result<Operand, Error> {
        self.read_operand_depth(0)
    }

    fn read_operand_depth(&mut self, depth: usize) -> Result<Operand, Error> {
        if depth > 4 {
            return Err(Error::Proto);
        }
        let value = self.read_tagged()?;
        match value.tag {
            TAG_UNSIGNED => Ok(Operand::Unsigned(value.value)),
            TAG_INTEGER => Ok(Operand::Integer(value.integer)),
            TAG_ATOM => {
                if value.value == 0 {
                    Ok(Operand::Nil)
                } else {
                    Ok(Operand::Atom(u32::try_from(value.value).map_err(|_| Error::TooBig)?))
                }
            }
            TAG_X_REGISTER => Ok(Operand::X(usize::try_from(value.value).map_err(|_| Error::TooBig)?)),
            TAG_Y_REGISTER => Ok(Operand::Y(usize::try_from(value.value).map_err(|_| Error::TooBig)?)),
            TAG_LABEL => Ok(Operand::Label(usize::try_from(value.value).map_err(|_| Error::TooBig)?)),
            TAG_EXTENDED => {
                self.skip_extended(value.value, depth + 1)?;
                Ok(Operand::Unsupported)
            }
            _ => Err(Error::NoExec),
        }
    }

    fn read_unsigned_tagged(&mut self, depth: usize) -> Result<u64, Error> {
        let value = self.read_tagged_depth(depth)?;
        if value.tag == TAG_UNSIGNED {
            Ok(value.value)
        } else {
            Err(Error::Proto)
        }
    }

    fn skip_extended(&mut self, extension: u64, depth: usize) -> Result<(), Error> {
        match extension {
            1 => {
                let count = usize::try_from(self.read_unsigned_tagged(depth)?)
                    .map_err(|_| Error::TooBig)?;
                if count > MAX_TABLE_ENTRIES {
                    return Err(Error::TooBig);
                }
                for _ in 0..count {
                    self.read_operand_depth(depth + 1)?;
                }
            }
            2 | 4 => {
                let _index = self.read_unsigned_tagged(depth)?;
            }
            3 => {
                let count = usize::try_from(self.read_unsigned_tagged(depth)?)
                    .map_err(|_| Error::TooBig)?;
                if count > 3 {
                    return Err(Error::Proto);
                }
                for _ in 0..count {
                    let _kind = self.read_unsigned_tagged(depth)?;
                    let _number = self.read_unsigned_tagged(depth)?;
                }
            }
            5 => {
                let register = self.read_tagged_depth(depth)?;
                if !matches!(register.tag, TAG_X_REGISTER | TAG_Y_REGISTER) {
                    return Err(Error::Proto);
                }
                let _type_index = self.read_unsigned_tagged(depth)?;
            }
            _ => return Err(Error::NoSys),
        }
        Ok(())
    }
}

fn parse_atoms(bytes: &[u8], utf8: bool) -> Result<Vec<Vec<u8>>, Error> {
    let mut cursor = Cursor::new(bytes);
    let count = read_cursor_u32(&mut cursor)? as usize;
    if count > MAX_TABLE_ENTRIES {
        return Err(Error::TooBig);
    }
    let mut atoms = Vec::new();
    atoms.try_reserve_exact(count + 1).map_err(|_| Error::NoMem)?;
    atoms.push(Vec::new());
    for _ in 0..count {
        let length = usize::from(cursor.read_u8()?);
        let name = cursor.read_bytes(length)?;
        if utf8 && core::str::from_utf8(name).is_err() {
            return Err(Error::Ilseq);
        }
        let mut owned = Vec::new();
        owned.try_reserve_exact(length).map_err(|_| Error::NoMem)?;
        owned.extend_from_slice(name);
        atoms.push(owned);
    }
    if cursor.remaining() != 0 {
        return Err(Error::Proto);
    }
    Ok(atoms)
}

fn parse_imports(bytes: &[u8]) -> Result<Vec<Import>, Error> {
    let mut cursor = Cursor::new(bytes);
    let count = read_cursor_u32(&mut cursor)? as usize;
    if count > MAX_TABLE_ENTRIES || count.checked_mul(12).ok_or(Error::TooBig)? > cursor.remaining() {
        return Err(Error::Proto);
    }
    let mut imports = Vec::new();
    imports.try_reserve_exact(count).map_err(|_| Error::NoMem)?;
    for _ in 0..count {
        imports.push(Import {
            module_atom: read_cursor_u32(&mut cursor)?,
            function_atom: read_cursor_u32(&mut cursor)?,
            arity: read_cursor_u32(&mut cursor)?,
        });
    }
    if cursor.remaining() != 0 {
        return Err(Error::Proto);
    }
    Ok(imports)
}

fn parse_exports(bytes: &[u8], atoms: &[Vec<u8>], label_count: usize) -> Result<Vec<Export>, Error> {
    let mut cursor = Cursor::new(bytes);
    let count = read_cursor_u32(&mut cursor)? as usize;
    if count > MAX_TABLE_ENTRIES || count.checked_mul(12).ok_or(Error::TooBig)? > cursor.remaining() {
        return Err(Error::Proto);
    }
    let mut exports = Vec::new();
    exports.try_reserve_exact(count).map_err(|_| Error::NoMem)?;
    for _ in 0..count {
        let function_atom = read_cursor_u32(&mut cursor)?;
        let arity = read_cursor_u32(&mut cursor)?;
        let label = usize::try_from(read_cursor_u32(&mut cursor)?).map_err(|_| Error::TooBig)?;
        if function_atom == 0
            || function_atom as usize >= atoms.len()
            || arity as usize > MAX_REGISTERS
            || label == 0
            || label >= label_count
        {
            return Err(Error::Proto);
        }
        exports.push(Export {
            function_atom,
            arity,
            label,
        });
    }
    if cursor.remaining() != 0 {
        return Err(Error::Proto);
    }
    Ok(exports)
}

fn opcode_arity(opcode: u8) -> Option<usize> {
    match opcode {
        1 => Some(1),
        2 => Some(3),
        3 | 19 => Some(0),
        4 => Some(2),
        5 => Some(3),
        6 | 7 => Some(2),
        8 => Some(3),
        9 => Some(2),
        10 => Some(4),
        11 => Some(5),
        12 => Some(2),
        13 => Some(3),
        14 => Some(2),
        15 => Some(3),
        16 => Some(2),
        17 | 18 => Some(1),
        20 | 21 | 22 => Some(0),
        23 => Some(2),
        24 | 25 => Some(1),
        26 => Some(2),
        27..=37 => Some(4),
        38 => Some(3),
        39..=44 => Some(3),
        45..=57 => Some(2),
        58..=60 => Some(3),
        61 => Some(1),
        62 => Some(2),
        63 => Some(1),
        64 => Some(2),
        65..=69 => Some(3),
        70 => Some(2),
        71 | 72 => Some(1),
        73 => Some(0),
        74 | 75 => Some(1),
        76 => Some(3),
        77 | 78 | 79 => Some(2),
        80..=82 => Some(5),
        83 => Some(4),
        84 => Some(2),
        85 | 86 => Some(1),
        87 | 88 => Some(2),
        89..=91 => Some(5),
        92 => Some(2),
        93 => Some(1),
        94 => Some(0),
        95 => Some(1),
        96 | 97 => Some(2),
        98..=101 => Some(4),
        102 => Some(3),
        103 => Some(1),
        104 => Some(2),
        105..=107 => Some(1),
        108 => Some(2),
        109 => Some(6),
        110 => Some(3),
        111 => Some(5),
        112 => Some(1),
        113 | 114 => Some(2),
        115 => Some(3),
        116 => Some(5),
        117..=119 => Some(7),
        120 => Some(5),
        121 => Some(3),
        122 | 123 => Some(2),
        124 => Some(5),
        125 => Some(6),
        126..=129 => Some(2),
        130 => Some(1),
        131 => Some(3),
        132 => Some(4),
        133 => Some(0),
        134 => Some(8),
        135 => Some(6),
        136 => Some(2),
        137 => Some(6),
        138 => Some(5),
        139 => Some(4),
        140 => Some(5),
        141 => Some(4),
        142 => Some(5),
        143 => Some(4),
        144..=148 => Some(3),
        149 => Some(0),
        150 | 151 => Some(1),
        152 => Some(7),
        153 => Some(1),
        154 | 155 => Some(5),
        156 => Some(2),
        157 | 158 => Some(3),
        159 => Some(4),
        160 | 161 => Some(0),
        162..=164 => Some(2),
        165 => Some(3),
        166 => Some(4),
        167 => Some(3),
        168 | 169 => Some(2),
        170 => Some(4),
        171 => Some(3),
        172 => Some(1),
        173 => Some(2),
        174..=176 => Some(1),
        177 => Some(6),
        178 => Some(3),
        179 => Some(0),
        180 => Some(1),
        181 => Some(5),
        182 => Some(3),
        183 => Some(2),
        _ => None,
    }
}

fn validate_operand(operand: Operand, atom_count: usize, label_count: usize) -> Result<(), Error> {
    match operand {
        Operand::Atom(index) if index as usize >= atom_count => Err(Error::Proto),
        Operand::X(index) if index >= MAX_REGISTERS => Err(Error::Proto),
        Operand::Y(index) if index >= MAX_STACK_WORDS => Err(Error::Proto),
        Operand::Label(index) if index >= label_count => Err(Error::Proto),
        _ => Ok(()),
    }
}

fn operand_unsigned(operand: Operand) -> Result<u64, Error> {
    match operand {
        Operand::Unsigned(value) => Ok(value),
        Operand::Integer(value) if value >= 0 => Ok(value as u64),
        _ => Err(Error::NoExec),
    }
}

fn operand_atom(operand: Operand) -> Result<u32, Error> {
    match operand {
        Operand::Atom(atom) => Ok(atom),
        _ => Err(Error::NoExec),
    }
}

fn operand_label(operand: Operand) -> Result<usize, Error> {
    match operand {
        Operand::Label(label) => Ok(label),
        _ => Err(Error::NoExec),
    }
}

fn operand_optional_label(operand: Operand) -> Result<Option<usize>, Error> {
    let label = operand_label(operand)?;
    Ok((label != 0).then_some(label))
}

fn operand_y(operand: Operand) -> Result<usize, Error> {
    match operand {
        Operand::Y(index) if index < MAX_STACK_WORDS => Ok(index),
        _ => Err(Error::NoExec),
    }
}

fn operand_source(operand: Operand, atoms: &[Vec<u8>]) -> Result<Source, Error> {
    match operand {
        Operand::Integer(value) => Ok(Source::Term(Term::Integer(value))),
        Operand::Atom(atom) if (atom as usize) < atoms.len() => Ok(Source::Term(Term::Atom(atom))),
        Operand::Nil => Ok(Source::Term(Term::Nil)),
        Operand::X(index) if index < MAX_REGISTERS => Ok(Source::X(index)),
        Operand::Y(index) if index < MAX_STACK_WORDS => Ok(Source::Y(index)),
        Operand::Unsigned(_) | Operand::Unsupported => Err(Error::NoSys),
        _ => Err(Error::NoExec),
    }
}

fn operand_destination(operand: Operand) -> Result<Destination, Error> {
    match operand {
        Operand::X(index) if index < MAX_REGISTERS => Ok(Destination::X(index)),
        Operand::Y(index) if index < MAX_STACK_WORDS => Ok(Destination::Y(index)),
        Operand::Unsupported => Err(Error::NoSys),
        _ => Err(Error::NoExec),
    }
}

fn push_decoded_instruction(
    instructions: &mut Vec<Instruction>,
    result: Result<Instruction, Error>,
) -> Result<(), Error> {
    match result {
        Ok(instruction) => instructions.push(instruction),
        Err(Error::NoSys) => instructions.push(Instruction::Unsupported),
        Err(error) => return Err(error),
    }
    Ok(())
}

fn import_bif1(index: u64, imports: &[Import], atoms: &[Vec<u8>]) -> Result<Bif1, Error> {
    let import = imports.get(usize::try_from(index).map_err(|_| Error::TooBig)?).ok_or(Error::NoExec)?;
    let module = atoms
        .get(import.module_atom as usize)
        .map(Vec::as_slice)
        .ok_or(Error::NoExec)?;
    if import.arity != 1 || module != &b"erlang"[..] {
        return Err(Error::NoSys);
    }
    let function = atoms
        .get(import.function_atom as usize)
        .map(Vec::as_slice)
        .ok_or(Error::NoExec)?;
    if function == &b"-"[..] {
        Ok(Bif1::Negate)
    } else if function == &b"abs"[..] {
        Ok(Bif1::Absolute)
    } else {
        Err(Error::NoSys)
    }
}

fn import_bif2(index: u64, imports: &[Import], atoms: &[Vec<u8>]) -> Result<Bif2, Error> {
    let import = imports.get(usize::try_from(index).map_err(|_| Error::TooBig)?).ok_or(Error::NoExec)?;
    let module = atoms
        .get(import.module_atom as usize)
        .map(Vec::as_slice)
        .ok_or(Error::NoExec)?;
    if import.arity != 2 || module != &b"erlang"[..] {
        return Err(Error::NoSys);
    }
    let function = atoms
        .get(import.function_atom as usize)
        .map(Vec::as_slice)
        .ok_or(Error::NoExec)?;
    if function == &b"+"[..] {
        Ok(Bif2::Add)
    } else if function == &b"-"[..] {
        Ok(Bif2::Subtract)
    } else if function == &b"*"[..] {
        Ok(Bif2::Multiply)
    } else if function == &b"div"[..] {
        Ok(Bif2::Divide)
    } else if function == &b"rem"[..] {
        Ok(Bif2::Remainder)
    } else if function == &b"band"[..] {
        Ok(Bif2::BitAnd)
    } else if function == &b"bor"[..] {
        Ok(Bif2::BitOr)
    } else if function == &b"bxor"[..] {
        Ok(Bif2::BitXor)
    } else {
        Err(Error::NoSys)
    }
}

fn parse_move(source: Operand, destination: Operand, atoms: &[Vec<u8>]) -> Result<Instruction, Error> {
    Ok(Instruction::Move {
        source: operand_source(source, atoms)?,
        destination: operand_destination(destination)?,
    })
}

fn parse_compare(
    fail: Operand,
    left: Operand,
    right: Operand,
    operation: Compare,
    atoms: &[Vec<u8>],
) -> Result<Instruction, Error> {
    Ok(Instruction::Compare {
        fail: operand_optional_label(fail)?,
        operation,
        left: operand_source(left, atoms)?,
        right: operand_source(right, atoms)?,
    })
}

fn parse_is_integer(fail: Operand, source: Operand, atoms: &[Vec<u8>]) -> Result<Instruction, Error> {
    Ok(Instruction::IsInteger {
        fail: operand_optional_label(fail)?,
        source: operand_source(source, atoms)?,
    })
}

fn parse_bif1(
    operands: [Operand; 8],
    imports: &[Import],
    atoms: &[Vec<u8>],
) -> Result<Instruction, Error> {
    let fail = operand_optional_label(operands[0])?;
    let _live = operand_unsigned(operands[1])?;
    let operation = import_bif1(operand_unsigned(operands[2])?, imports, atoms)?;
    Ok(Instruction::Bif1 {
        fail,
        operation,
        source: operand_source(operands[3], atoms)?,
        destination: operand_destination(operands[4])?,
    })
}

fn parse_bif2(
    operands: [Operand; 8],
    imports: &[Import],
    atoms: &[Vec<u8>],
) -> Result<Instruction, Error> {
    let fail = operand_optional_label(operands[0])?;
    let _live = operand_unsigned(operands[1])?;
    let operation = import_bif2(operand_unsigned(operands[2])?, imports, atoms)?;
    Ok(Instruction::Bif2 {
        fail,
        operation,
        left: operand_source(operands[3], atoms)?,
        right: operand_source(operands[4], atoms)?,
        destination: operand_destination(operands[5])?,
    })
}

fn read_source(
    source: Source,
    registers: &[Term; MAX_REGISTERS],
    stack: &[Term; MAX_STACK_WORDS],
    stack_length: usize,
) -> Result<Term, Error> {
    match source {
        Source::Term(term) => Ok(term),
        Source::X(index) => registers.get(index).copied().ok_or(Error::NoExec),
        Source::Y(index) if index < stack_length => stack.get(index).copied().ok_or(Error::NoExec),
        Source::Y(_) => Err(Error::NoExec),
    }
}

fn write_destination(
    destination: Destination,
    value: Term,
    registers: &mut [Term; MAX_REGISTERS],
    stack: &mut [Term; MAX_STACK_WORDS],
    stack_length: usize,
) -> Result<(), Error> {
    match destination {
        Destination::X(index) => {
            *registers.get_mut(index).ok_or(Error::NoExec)? = value;
            Ok(())
        }
        Destination::Y(index) if index < stack_length => {
            *stack.get_mut(index).ok_or(Error::NoExec)? = value;
            Ok(())
        }
        Destination::Y(_) => Err(Error::NoExec),
    }
}

fn integer_operand(term: Term) -> Result<i64, Error> {
    match term {
        Term::Integer(value) => Ok(value),
        Term::Atom(_) | Term::Nil => Err(Error::Inval),
    }
}

fn execute_bif1(operation: Bif1, source: Term) -> Result<Term, Error> {
    let value = integer_operand(source)?;
    let result = match operation {
        Bif1::Negate => value.checked_neg().ok_or(Error::Overflow)?,
        Bif1::Absolute => value.checked_abs().ok_or(Error::Overflow)?,
    };
    Ok(Term::Integer(result))
}

fn execute_bif2(operation: Bif2, left: Term, right: Term) -> Result<Term, Error> {
    let left = integer_operand(left)?;
    let right = integer_operand(right)?;
    let result = match operation {
        Bif2::Add => left.checked_add(right).ok_or(Error::Overflow)?,
        Bif2::Subtract => left.checked_sub(right).ok_or(Error::Overflow)?,
        Bif2::Multiply => left.checked_mul(right).ok_or(Error::Overflow)?,
        Bif2::Divide => left.checked_div(right).ok_or(if right == 0 { Error::Dom } else { Error::Overflow })?,
        Bif2::Remainder => left.checked_rem(right).ok_or(if right == 0 { Error::Dom } else { Error::Overflow })?,
        Bif2::BitAnd => left & right,
        Bif2::BitOr => left | right,
        Bif2::BitXor => left ^ right,
    };
    Ok(Term::Integer(result))
}

fn compare_terms(operation: Compare, left: Term, right: Term) -> Result<bool, Error> {
    match operation {
        Compare::Equal | Compare::ExactEqual => Ok(left == right),
        Compare::NotEqual | Compare::ExactNotEqual => Ok(left != right),
        Compare::LessThan | Compare::GreaterOrEqual => {
            let left = integer_operand(left)?;
            let right = integer_operand(right)?;
            Ok(match operation {
                Compare::LessThan => left < right,
                Compare::GreaterOrEqual => left >= right,
                _ => unreachable!(),
            })
        }
    }
}

fn parse_code(
    bytes: &[u8],
    atoms: &[Vec<u8>],
    imports: &[Import],
) -> Result<(CodeHeader, Vec<Instruction>, Vec<Option<usize>>), Error> {
    let header_size = read_be_u32(bytes, 0)? as usize;
    if header_size < 16 || header_size > 64 {
        return Err(Error::NoExec);
    }
    let code_start = 4usize.checked_add(header_size).ok_or(Error::Proto)?;
    if code_start > bytes.len() || bytes.len() - code_start > MAX_CODE_BYTES {
        return Err(Error::TooBig);
    }
    let instruction_set = read_be_u32(bytes, 4)?;
    let max_opcode = read_be_u32(bytes, 8)?;
    let label_count = read_be_u32(bytes, 12)? as usize;
    let function_count = read_be_u32(bytes, 16)? as usize;
    if instruction_set != BEAM_FORMAT_NUMBER
        || max_opcode > MAX_GENERIC_OPCODE
        || label_count == 0
        || label_count > MAX_INSTRUCTIONS
        || function_count > MAX_INSTRUCTIONS
    {
        return Err(Error::NoExec);
    }

    let mut cursor = Cursor::new(&bytes[code_start..]);
    let mut instructions = Vec::new();
    instructions
        .try_reserve_exact(((bytes.len() - code_start) / 2).min(256))
        .map_err(|_| Error::NoMem)?;
    let mut labels = Vec::new();
    labels.try_reserve_exact(label_count).map_err(|_| Error::NoMem)?;
    labels.resize(label_count, None);
    let mut seen_functions = 0usize;
    let mut saw_end = false;

    while cursor.remaining() != 0 {
        if instructions.len() >= MAX_INSTRUCTIONS {
            return Err(Error::TooBig);
        }
        if instructions.len() == instructions.capacity() {
            instructions.try_reserve(1).map_err(|_| Error::NoMem)?;
        }
        let opcode = cursor.read_u8()?;
        let arity = opcode_arity(opcode).ok_or(Error::NoSys)?;
        let mut operands = [Operand::Unsigned(0); 8];
        for operand in operands.iter_mut().take(arity) {
            *operand = cursor.read_operand()?;
        }
        for operand in operands.iter().take(arity) {
            validate_operand(*operand, atoms.len(), label_count)?;
        }
        match opcode {
            1 => {
                let label = operand_label(operands[0])?;
                if label == 0 || label >= label_count || labels[label].is_some() {
                    return Err(Error::Proto);
                }
                labels[label] = Some(instructions.len());
                instructions.push(Instruction::Label(label));
            }
            2 => {
                let module_atom = operand_atom(operands[0])?;
                let function_atom = operand_atom(operands[1])?;
                let function_arity = operand_unsigned(operands[2])?;
                if module_atom as usize >= atoms.len()
                    || function_atom as usize >= atoms.len()
                    || function_arity as usize > MAX_REGISTERS
                {
                    return Err(Error::Proto);
                }
                seen_functions += 1;
            }
            3 => {
                saw_end = true;
                break;
            }
            12 => {
                let words = usize::try_from(operand_unsigned(operands[0])?).map_err(|_| Error::TooBig)?;
                let _live = operand_unsigned(operands[1])?;
                if words > MAX_STACK_WORDS {
                    return Err(Error::TooBig);
                }
                instructions.push(Instruction::Allocate(words));
            }
            16 | 153 => instructions.push(Instruction::Noop),
            17 => instructions.push(Instruction::InitializeY(operand_y(operands[0])?)),
            18 => instructions.push(Instruction::Deallocate),
            19 => instructions.push(Instruction::Return),
            39..=44 => {
                let operation = match opcode {
                    39 => Compare::LessThan,
                    40 => Compare::GreaterOrEqual,
                    41 => Compare::Equal,
                    42 => Compare::NotEqual,
                    43 => Compare::ExactEqual,
                    _ => Compare::ExactNotEqual,
                };
                push_decoded_instruction(
                    &mut instructions,
                    parse_compare(operands[0], operands[1], operands[2], operation, atoms),
                )?;
            }
            45 => push_decoded_instruction(
                &mut instructions,
                parse_is_integer(operands[0], operands[1], atoms),
            )?,
            61 => instructions.push(Instruction::Jump(operand_label(operands[0])?)),
            64 => push_decoded_instruction(
                &mut instructions,
                parse_move(operands[0], operands[1], atoms),
            )?,
            124 => push_decoded_instruction(
                &mut instructions,
                parse_bif1(operands, imports, atoms),
            )?,
            125 => push_decoded_instruction(
                &mut instructions,
                parse_bif2(operands, imports, atoms),
            )?,
            _ => instructions.push(Instruction::Unsupported),
        }
    }

    if !saw_end || cursor.remaining() != 0 || seen_functions != function_count {
        return Err(Error::Proto);
    }
    for instruction in &instructions {
        match instruction {
            Instruction::Jump(label) => {
                if *label == 0 || labels.get(*label).and_then(|target| *target).is_none() {
                    return Err(Error::NoExec);
                }
            }
            Instruction::Compare { fail: Some(label), .. }
            | Instruction::IsInteger { fail: Some(label), .. } => {
                if labels.get(*label).and_then(|target| *target).is_none() {
                    return Err(Error::NoExec);
                }
            }
            Instruction::Bif1 { fail: Some(label), .. }
            | Instruction::Bif2 { fail: Some(label), .. } => {
                if labels.get(*label).and_then(|target| *target).is_none() {
                    return Err(Error::NoExec);
                }
            }
            _ => {}
        }
    }
    Ok((CodeHeader { label_count }, instructions, labels))
}

fn read_cursor_u32(cursor: &mut Cursor<'_>) -> Result<u32, Error> {
    let bytes = cursor.read_bytes(4)?;
    Ok(u32::from_be_bytes(bytes.try_into().map_err(|_| Error::Proto)?))
}

fn read_be_u32(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let end = offset.checked_add(4).ok_or(Error::Proto)?;
    let value = bytes.get(offset..end).ok_or(Error::Proto)?;
    Ok(u32::from_be_bytes(value.try_into().map_err(|_| Error::Proto)?))
}
