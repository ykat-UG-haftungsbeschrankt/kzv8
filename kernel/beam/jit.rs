/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::mem::size_of;
use core::sync::atomic::{fence, Ordering};

use crate::kernel::error::Error;

use super::{
    Bif1, Bif2, Compare, Destination, Instruction, Source, Term, MAX_EXECUTION_STEPS,
    MAX_REGISTERS, MAX_STACK_WORDS,
};

const PAGE_SIZE: usize = crate::kernel::memory::r#virtual::PAGE_SIZE;
const MAX_NATIVE_CODE_SIZE: usize = 64 * 1024;
const MAX_SINGLE_INSTRUCTION_CODE_SIZE: usize = 8 * 1024;

const JIT_TAG_INTEGER: u64 = 0;
const JIT_TAG_ATOM: u64 = 1;
const JIT_TAG_NIL: u64 = 2;

const JIT_STATUS_OVERFLOW: u32 = 1;
const JIT_STATUS_INVAL: u32 = 2;
const JIT_STATUS_DOM: u32 = 3;
const JIT_STATUS_NO_EXEC: u32 = 4;
const JIT_STATUS_TIMED_OUT: u32 = 5;

const REG_RAX: u8 = 0;
const REG_RCX: u8 = 1;
const REG_RDX: u8 = 2;
const REG_R8: u8 = 8;
const REG_R9: u8 = 9;

const JCC_OVERFLOW: u8 = 0x80;
const JCC_NOT_EQUAL: u8 = 0x85;
const JCC_EQUAL: u8 = 0x84;
const JCC_BELOW_OR_EQUAL: u8 = 0x86;
const JCC_ABOVE: u8 = 0x87;
const JCC_NOT_SIGN: u8 = 0x89;
const JCC_LESS: u8 = 0x8c;
const JCC_GREATER_OR_EQUAL: u8 = 0x8d;

#[repr(C)]
#[derive(Clone, Copy)]
struct JitValue {
    value: i64,
    tag: u64,
}

impl JitValue {
    const NIL: Self = Self {
        value: 0,
        tag: JIT_TAG_NIL,
    };

    fn from_term(term: Term) -> Self {
        match term {
            Term::Integer(value) => Self {
                value,
                tag: JIT_TAG_INTEGER,
            },
            Term::Atom(atom) => Self {
                value: i64::from(atom),
                tag: JIT_TAG_ATOM,
            },
            Term::Nil => Self::NIL,
        }
    }
}

#[repr(C)]
struct JitFrame {
    registers: [JitValue; MAX_REGISTERS],
    stack: [JitValue; MAX_STACK_WORDS],
    stack_length: usize,
    steps: usize,
    status: u32,
    padding: u32,
    result_tag: u64,
}

const STACK_OFFSET: usize = MAX_REGISTERS * size_of::<JitValue>();
const STACK_LENGTH_OFFSET: usize = STACK_OFFSET + MAX_STACK_WORDS * size_of::<JitValue>();
const STEPS_OFFSET: usize = STACK_LENGTH_OFFSET + size_of::<usize>();
const STATUS_OFFSET: usize = STEPS_OFFSET + size_of::<usize>();
const RESULT_TAG_OFFSET: usize = STATUS_OFFSET + 8;

pub(crate) struct JitFunction {
    address: usize,
    allocation_length: usize,
    arity: usize,
}

impl JitFunction {
    pub(super) fn compile(
        instructions: &[Instruction],
        labels: &[Option<usize>],
        start: usize,
        arity: usize,
    ) -> Result<Self, Error> {
        if arity > MAX_REGISTERS {
            return Err(Error::TooBig);
        }
        let code = emit_function(instructions, labels, start)?;
        if code.is_empty() || code.len() > MAX_NATIVE_CODE_SIZE {
            return Err(Error::TooBig);
        }
        let allocation_length = code
            .len()
            .checked_add(PAGE_SIZE - 1)
            .map(|length| length & !(PAGE_SIZE - 1))
            .ok_or(Error::Overflow)?;
        let address = crate::kernel::memory::r#virtual::reserve(
            allocation_length,
            PAGE_SIZE,
            2,
        )
        .ok_or(Error::NoMem)?;

        // SAFETY: reserve returned a writable kernel mapping covering the generated code.
        unsafe {
            core::ptr::copy_nonoverlapping(code.as_ptr(), address as *mut u8, code.len());
        }
        fence(Ordering::SeqCst);
        if !crate::kernel::memory::r#virtual::set_permissions(address, allocation_length, 4) {
            let _released = crate::kernel::memory::r#virtual::release(address, allocation_length);
            return Err(Error::NoMem);
        }
        synchronize_instruction_cache();
        Ok(Self {
            address,
            allocation_length,
            arity,
        })
    }

    pub(crate) fn run(&self, arguments: &[Term]) -> Result<Term, Error> {
        if arguments.len() != self.arity {
            return Err(Error::Inval);
        }
        let nil = JitValue::NIL;
        let mut frame = JitFrame {
            registers: [nil; MAX_REGISTERS],
            stack: [nil; MAX_STACK_WORDS],
            stack_length: 0,
            steps: 0,
            status: 0,
            padding: 0,
            result_tag: JIT_TAG_NIL,
        };
        for (register, argument) in frame.registers.iter_mut().zip(arguments) {
            *register = JitValue::from_term(*argument);
        }

        // SAFETY: the RX mapping contains code emitted for the verified instruction subset; its
        // ABI is `extern "C" fn(*mut JitFrame) -> i64`, and the frame remains live for the call.
        let entry: unsafe extern "C" fn(*mut JitFrame) -> i64 =
            unsafe { core::mem::transmute(self.address) };
        let result = unsafe { entry(&mut frame) };
        match frame.status {
            0 => match frame.result_tag {
                JIT_TAG_INTEGER => Ok(Term::Integer(result)),
                JIT_TAG_ATOM => u32::try_from(result)
                    .map(Term::Atom)
                    .map_err(|_| Error::Error),
                JIT_TAG_NIL => Ok(Term::Nil),
                _ => Err(Error::Error),
            },
            JIT_STATUS_OVERFLOW => Err(Error::Overflow),
            JIT_STATUS_INVAL => Err(Error::Inval),
            JIT_STATUS_DOM => Err(Error::Dom),
            JIT_STATUS_NO_EXEC => Err(Error::NoExec),
            JIT_STATUS_TIMED_OUT => Err(Error::TimedOut),
            _ => Err(Error::Error),
        }
    }
}

impl Drop for JitFunction {
    fn drop(&mut self) {
        let _released =
            crate::kernel::memory::r#virtual::release(self.address, self.allocation_length);
    }
}

#[derive(Clone, Copy)]
enum BranchTarget {
    Instruction(usize),
    Error(u32),
    CodeOffset(usize),
}

struct Fixup {
    displacement: usize,
    target: BranchTarget,
}

fn emit_function(
    instructions: &[Instruction],
    labels: &[Option<usize>],
    start: usize,
) -> Result<Vec<u8>, Error> {
    let reachable = collect_reachable(instructions, labels, start)?;
    let mut code = Vec::new();
    code.try_reserve_exact(MAX_NATIVE_CODE_SIZE + MAX_SINGLE_INSTRUCTION_CODE_SIZE)
        .map_err(|_| Error::NoMem)?;
    let mut instruction_offsets = Vec::new();
    instruction_offsets
        .try_reserve_exact(instructions.len())
        .map_err(|_| Error::NoMem)?;
    instruction_offsets.resize(instructions.len(), usize::MAX);
    let mut fixups = Vec::new();
    fixups
        .try_reserve_exact(MAX_NATIVE_CODE_SIZE / 5)
        .map_err(|_| Error::NoMem)?;

    emit_jump(
        &mut code,
        BranchTarget::Instruction(start),
        &mut fixups,
    )?;
    for (pc, instruction) in instructions.iter().copied().enumerate() {
        if !reachable[pc] {
            continue;
        }
        instruction_offsets[pc] = code.len();
        emit_step_check(&mut code, &mut fixups)?;
        emit_instruction(&mut code, instruction, labels, &mut fixups)?;
        if pc + 1 == instructions.len()
            && !matches!(instruction, Instruction::Jump(_) | Instruction::Return)
        {
            emit_jump(
                &mut code,
                BranchTarget::Error(JIT_STATUS_NO_EXEC),
                &mut fixups,
            )?;
        }
        if code.len() > MAX_NATIVE_CODE_SIZE {
            return Err(Error::TooBig);
        }
    }

    let error_offsets = append_error_handlers(&mut code)?;
    if code.len() > MAX_NATIVE_CODE_SIZE {
        return Err(Error::TooBig);
    }
    patch_fixups(&mut code, &fixups, &instruction_offsets, &error_offsets)?;
    Ok(code)
}

fn collect_reachable(
    instructions: &[Instruction],
    labels: &[Option<usize>],
    start: usize,
) -> Result<Vec<bool>, Error> {
    if start >= instructions.len() {
        return Err(Error::NoExec);
    }
    let mut reachable = Vec::new();
    reachable
        .try_reserve_exact(instructions.len())
        .map_err(|_| Error::NoMem)?;
    reachable.resize(instructions.len(), false);

    let pending_capacity = instructions
        .len()
        .checked_mul(2)
        .and_then(|capacity| capacity.checked_add(1))
        .ok_or(Error::Overflow)?;
    let mut pending = Vec::new();
    pending
        .try_reserve_exact(pending_capacity)
        .map_err(|_| Error::NoMem)?;
    pending.push(start);

    while let Some(pc) = pending.pop() {
        if pc >= instructions.len() {
            return Err(Error::NoExec);
        }
        if reachable[pc] {
            continue;
        }
        reachable[pc] = true;
        match instructions[pc] {
            Instruction::Unsupported => return Err(Error::NoSys),
            Instruction::Return => {}
            Instruction::Jump(label) => {
                pending.push(resolve_label(labels, label, instructions.len())?);
            }
            Instruction::Bif1 { fail, .. }
            | Instruction::Bif2 { fail, .. }
            | Instruction::Compare { fail, .. }
            | Instruction::IsInteger { fail, .. } => {
                if let Some(label) = fail {
                    pending.push(resolve_label(labels, label, instructions.len())?);
                }
                if pc + 1 < instructions.len() {
                    pending.push(pc + 1);
                }
            }
            _ if pc + 1 < instructions.len() => pending.push(pc + 1),
            _ => {}
        }
    }
    Ok(reachable)
}

fn resolve_label(
    labels: &[Option<usize>],
    label: usize,
    instruction_count: usize,
) -> Result<usize, Error> {
    labels
        .get(label)
        .copied()
        .flatten()
        .filter(|target| *target < instruction_count)
        .ok_or(Error::NoExec)
}

fn emit_instruction(
    code: &mut Vec<u8>,
    instruction: Instruction,
    labels: &[Option<usize>],
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    match instruction {
        Instruction::Noop | Instruction::Label(_) => Ok(()),
        Instruction::Move {
            source,
            destination,
        } => {
            emit_load_source(
                code,
                source,
                REG_RAX,
                REG_RDX,
                BranchTarget::Error(JIT_STATUS_NO_EXEC),
                fixups,
            )?;
            emit_store_destination(
                code,
                destination,
                REG_RAX,
                REG_RDX,
                fixups,
            )
        }
        Instruction::Bif1 {
            fail,
            operation,
            source,
            destination,
        } => emit_bif1(code, fail, operation, source, destination, labels, fixups),
        Instruction::Bif2 {
            fail,
            operation,
            left,
            right,
            destination,
        } => emit_bif2(
            code,
            fail,
            operation,
            left,
            right,
            destination,
            labels,
            fixups,
        ),
        Instruction::Compare {
            fail,
            operation,
            left,
            right,
        } => emit_compare(code, fail, operation, left, right, labels, fixups),
        Instruction::IsInteger { fail, source } => {
            let invalid_source = BranchTarget::Error(JIT_STATUS_NO_EXEC);
            emit_load_source(code, source, REG_RAX, REG_RDX, invalid_source, fixups)?;
            let failure = failure_target(fail, JIT_STATUS_INVAL, labels)?;
            emit_compare_register_immediate(code, REG_RDX, JIT_TAG_INTEGER as i8)?;
            emit_conditional_jump(code, JCC_NOT_EQUAL, failure, fixups)?;
            Ok(())
        }
        Instruction::Jump(label) => {
            let target = resolve_label(labels, label, usize::MAX)?;
            emit_jump(code, BranchTarget::Instruction(target), fixups)
        }
        Instruction::Allocate(words) => emit_allocate(code, words),
        Instruction::Deallocate => emit_store_frame_immediate(code, STACK_LENGTH_OFFSET, 0),
        Instruction::InitializeY(index) => emit_initialize_y(code, index, fixups),
        Instruction::Return => emit_return(code),
        Instruction::Unsupported => Err(Error::NoSys),
    }
}

fn emit_bif1(
    code: &mut Vec<u8>,
    fail: Option<usize>,
    operation: Bif1,
    source: Source,
    destination: Destination,
    labels: &[Option<usize>],
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    let source_failure = failure_target(fail, JIT_STATUS_NO_EXEC, labels)?;
    emit_load_source(code, source, REG_RAX, REG_RDX, source_failure, fixups)?;

    let invalid = failure_target(fail, JIT_STATUS_INVAL, labels)?;
    emit_compare_register_immediate(code, REG_RDX, JIT_TAG_INTEGER as i8)?;
    emit_conditional_jump(code, JCC_NOT_EQUAL, invalid, fixups)?;

    let overflow = failure_target(fail, JIT_STATUS_OVERFLOW, labels)?;
    match operation {
        Bif1::Negate => {
            code.extend_from_slice(&[0x48, 0xf7, 0xd8]);
            emit_conditional_jump(code, JCC_OVERFLOW, overflow, fixups)?;
        }
        Bif1::Absolute => {
            code.extend_from_slice(&[0x48, 0x85, 0xc0]);
            let non_negative = emit_conditional_jump(
                code,
                JCC_NOT_SIGN,
                BranchTarget::CodeOffset(0),
                fixups,
            )?;
            code.extend_from_slice(&[0x48, 0xf7, 0xd8]);
            emit_conditional_jump(code, JCC_OVERFLOW, overflow, fixups)?;
            set_code_offset(fixups, non_negative, code.len())?;
        }
    }

    code.extend_from_slice(&[0x31, 0xd2]);
    emit_store_destination(code, destination, REG_RAX, REG_RDX, fixups)
}

fn emit_bif2(
    code: &mut Vec<u8>,
    fail: Option<usize>,
    operation: Bif2,
    left: Source,
    right: Source,
    destination: Destination,
    labels: &[Option<usize>],
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    let source_failure = failure_target(fail, JIT_STATUS_NO_EXEC, labels)?;
    emit_load_source(code, left, REG_RAX, REG_RDX, source_failure, fixups)?;
    emit_load_source(code, right, REG_RCX, REG_R8, source_failure, fixups)?;

    let invalid = failure_target(fail, JIT_STATUS_INVAL, labels)?;
    emit_compare_register_immediate(code, REG_RDX, JIT_TAG_INTEGER as i8)?;
    emit_conditional_jump(code, JCC_NOT_EQUAL, invalid, fixups)?;
    emit_compare_register_immediate(code, REG_R8, JIT_TAG_INTEGER as i8)?;
    emit_conditional_jump(code, JCC_NOT_EQUAL, invalid, fixups)?;

    match operation {
        Bif2::Add => {
            emit_binary(code, 0x01, REG_RAX, REG_RCX);
            let overflow = failure_target(fail, JIT_STATUS_OVERFLOW, labels)?;
            emit_conditional_jump(code, JCC_OVERFLOW, overflow, fixups)?;
        }
        Bif2::Subtract => {
            emit_binary(code, 0x29, REG_RAX, REG_RCX);
            let overflow = failure_target(fail, JIT_STATUS_OVERFLOW, labels)?;
            emit_conditional_jump(code, JCC_OVERFLOW, overflow, fixups)?;
        }
        Bif2::Multiply => {
            code.extend_from_slice(&[0x48, 0x0f, 0xaf, 0xc1]);
            let overflow = failure_target(fail, JIT_STATUS_OVERFLOW, labels)?;
            emit_conditional_jump(code, JCC_OVERFLOW, overflow, fixups)?;
        }
        Bif2::Divide | Bif2::Remainder => {
            emit_compare_register_immediate(code, REG_RCX, 0)?;
            let division_by_zero = failure_target(fail, JIT_STATUS_DOM, labels)?;
            emit_conditional_jump(code, JCC_EQUAL, division_by_zero, fixups)?;

            emit_load_immediate(code, REG_R9, i64::MIN)?;
            emit_compare_registers(code, REG_RAX, REG_R9);
            let not_minimum = emit_conditional_jump(
                code,
                JCC_NOT_EQUAL,
                BranchTarget::CodeOffset(0),
                fixups,
            )?;
            emit_compare_register_immediate(code, REG_RCX, -1)?;
            let overflow = failure_target(fail, JIT_STATUS_OVERFLOW, labels)?;
            emit_conditional_jump(code, JCC_EQUAL, overflow, fixups)?;
            set_code_offset(fixups, not_minimum, code.len())?;

            code.extend_from_slice(&[0x48, 0x99, 0x48, 0xf7, 0xf9]);
            if operation == Bif2::Remainder {
                emit_move_register(code, REG_RAX, REG_RDX);
            }
        }
        Bif2::BitAnd => emit_binary(code, 0x21, REG_RAX, REG_RCX),
        Bif2::BitOr => emit_binary(code, 0x09, REG_RAX, REG_RCX),
        Bif2::BitXor => emit_binary(code, 0x31, REG_RAX, REG_RCX),
    }

    code.extend_from_slice(&[0x31, 0xd2]);
    emit_store_destination(code, destination, REG_RAX, REG_RDX, fixups)
}

fn emit_compare(
    code: &mut Vec<u8>,
    fail: Option<usize>,
    operation: Compare,
    left: Source,
    right: Source,
    labels: &[Option<usize>],
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    let invalid_source = BranchTarget::Error(JIT_STATUS_NO_EXEC);
    emit_load_source(code, left, REG_RAX, REG_RDX, invalid_source, fixups)?;
    emit_load_source(code, right, REG_RCX, REG_R8, invalid_source, fixups)?;
    let failure = failure_target(fail, JIT_STATUS_INVAL, labels)?;

    match operation {
        Compare::Equal | Compare::ExactEqual => {
            emit_compare_registers(code, REG_RDX, REG_R8);
            emit_conditional_jump(code, JCC_NOT_EQUAL, failure, fixups)?;
            emit_compare_registers(code, REG_RAX, REG_RCX);
            emit_conditional_jump(code, JCC_NOT_EQUAL, failure, fixups)?;
        }
        Compare::NotEqual | Compare::ExactNotEqual => {
            emit_compare_registers(code, REG_RDX, REG_R8);
            let tags_differ = emit_conditional_jump(
                code,
                JCC_NOT_EQUAL,
                BranchTarget::CodeOffset(0),
                fixups,
            )?;
            emit_compare_registers(code, REG_RAX, REG_RCX);
            emit_conditional_jump(code, JCC_EQUAL, failure, fixups)?;
            set_code_offset(fixups, tags_differ, code.len())?;
        }
        Compare::LessThan | Compare::GreaterOrEqual => {
            emit_compare_register_immediate(code, REG_RDX, JIT_TAG_INTEGER as i8)?;
            emit_conditional_jump(
                code,
                JCC_NOT_EQUAL,
                BranchTarget::Error(JIT_STATUS_INVAL),
                fixups,
            )?;
            emit_compare_register_immediate(code, REG_R8, JIT_TAG_INTEGER as i8)?;
            emit_conditional_jump(
                code,
                JCC_NOT_EQUAL,
                BranchTarget::Error(JIT_STATUS_INVAL),
                fixups,
            )?;
            emit_compare_registers(code, REG_RAX, REG_RCX);
            let condition = match operation {
                Compare::LessThan => JCC_GREATER_OR_EQUAL,
                Compare::GreaterOrEqual => JCC_LESS,
                _ => unreachable!(),
            };
            emit_conditional_jump(code, condition, failure, fixups)?;
        }
    }
    Ok(())
}

fn emit_load_source(
    code: &mut Vec<u8>,
    source: Source,
    value_register: u8,
    tag_register: u8,
    invalid_stack: BranchTarget,
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    match source {
        Source::Term(term) => {
            let value = match term {
                Term::Integer(value) => value,
                Term::Atom(atom) => i64::from(atom),
                Term::Nil => 0,
            };
            let tag = match term {
                Term::Integer(_) => JIT_TAG_INTEGER,
                Term::Atom(_) => JIT_TAG_ATOM,
                Term::Nil => JIT_TAG_NIL,
            };
            emit_load_immediate(code, value_register, value)?;
            emit_load_immediate(code, tag_register, tag as i64)
        }
        Source::X(index) => {
            if index >= MAX_REGISTERS {
                return Err(Error::NoExec);
            }
            emit_load_frame(
                code,
                value_register,
                slot_offset(0, index, 0)?,
            )?;
            emit_load_frame(
                code,
                tag_register,
                slot_offset(0, index, size_of::<i64>())?,
            )
        }
        Source::Y(index) => {
            if index >= MAX_STACK_WORDS {
                return Err(Error::NoExec);
            }
            emit_stack_index_check(code, index, invalid_stack, fixups)?;
            emit_load_frame(
                code,
                value_register,
                slot_offset(STACK_OFFSET, index, 0)?,
            )?;
            emit_load_frame(
                code,
                tag_register,
                slot_offset(STACK_OFFSET, index, size_of::<i64>())?,
            )
        }
    }
}

fn emit_store_destination(
    code: &mut Vec<u8>,
    destination: Destination,
    value_register: u8,
    tag_register: u8,
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    let (base, index) = match destination {
        Destination::X(index) if index < MAX_REGISTERS => (0, index),
        Destination::Y(index) if index < MAX_STACK_WORDS => {
            emit_stack_index_check(
                code,
                index,
                BranchTarget::Error(JIT_STATUS_NO_EXEC),
                fixups,
            )?;
            (STACK_OFFSET, index)
        }
        _ => return Err(Error::NoExec),
    };
    emit_store_frame(
        code,
        value_register,
        slot_offset(base, index, 0)?,
    )?;
    emit_store_frame(
        code,
        tag_register,
        slot_offset(base, index, size_of::<i64>())?,
    )
}

fn emit_stack_index_check(
    code: &mut Vec<u8>,
    index: usize,
    invalid: BranchTarget,
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    let index = i32::try_from(index).map_err(|_| Error::Overflow)?;
    emit_compare_frame_immediate(code, STACK_LENGTH_OFFSET, index)?;
    emit_conditional_jump(code, JCC_BELOW_OR_EQUAL, invalid, fixups)?;
    Ok(())
}

fn emit_allocate(code: &mut Vec<u8>, words: usize) -> Result<(), Error> {
    if words > MAX_STACK_WORDS {
        return Err(Error::TooBig);
    }
    for index in 0..words {
        emit_store_frame_immediate(code, slot_offset(STACK_OFFSET, index, 0)?, 0)?;
        emit_store_frame_immediate(
            code,
            slot_offset(STACK_OFFSET, index, size_of::<i64>())?,
            JIT_TAG_NIL as i32,
        )?;
    }
    emit_store_frame_immediate(
        code,
        STACK_LENGTH_OFFSET,
        i32::try_from(words).map_err(|_| Error::Overflow)?,
    )
}

fn emit_initialize_y(
    code: &mut Vec<u8>,
    index: usize,
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    if index >= MAX_STACK_WORDS {
        return Err(Error::NoExec);
    }
    emit_stack_index_check(
        code,
        index,
        BranchTarget::Error(JIT_STATUS_NO_EXEC),
        fixups,
    )?;
    emit_store_frame_immediate(code, slot_offset(STACK_OFFSET, index, 0)?, 0)?;
    emit_store_frame_immediate(
        code,
        slot_offset(STACK_OFFSET, index, size_of::<i64>())?,
        JIT_TAG_NIL as i32,
    )
}

fn emit_return(code: &mut Vec<u8>) -> Result<(), Error> {
    emit_load_frame(code, REG_RAX, slot_offset(0, 0, 0)?)?;
    emit_load_frame(
        code,
        REG_RDX,
        slot_offset(0, 0, size_of::<i64>())?,
    )?;
    emit_store_frame(code, REG_RDX, RESULT_TAG_OFFSET)?;
    code.push(0xc3);
    Ok(())
}

fn emit_step_check(code: &mut Vec<u8>, fixups: &mut Vec<Fixup>) -> Result<(), Error> {
    emit_increment_frame(code, STEPS_OFFSET)?;
    emit_compare_frame_immediate(
        code,
        STEPS_OFFSET,
        i32::try_from(MAX_EXECUTION_STEPS).map_err(|_| Error::Overflow)?,
    )?;
    emit_conditional_jump(
        code,
        JCC_ABOVE,
        BranchTarget::Error(JIT_STATUS_TIMED_OUT),
        fixups,
    )?;
    Ok(())
}

fn failure_target(
    fail: Option<usize>,
    status: u32,
    labels: &[Option<usize>],
) -> Result<BranchTarget, Error> {
    match fail {
        Some(label) => Ok(BranchTarget::Instruction(resolve_label(
            labels,
            label,
            usize::MAX,
        )?)),
        None => Ok(BranchTarget::Error(status)),
    }
}

fn append_error_handlers(code: &mut Vec<u8>) -> Result<[usize; 6], Error> {
    let mut offsets = [usize::MAX; 6];
    for status in [
        JIT_STATUS_OVERFLOW,
        JIT_STATUS_INVAL,
        JIT_STATUS_DOM,
        JIT_STATUS_NO_EXEC,
        JIT_STATUS_TIMED_OUT,
    ] {
        offsets[status as usize] = code.len();
        emit_store_status(code, status)?;
        code.extend_from_slice(&[0x31, 0xc0, 0xc3]);
    }
    Ok(offsets)
}

fn patch_fixups(
    code: &mut [u8],
    fixups: &[Fixup],
    instruction_offsets: &[usize],
    error_offsets: &[usize; 6],
) -> Result<(), Error> {
    for fixup in fixups {
        let target = match fixup.target {
            BranchTarget::Instruction(pc) => instruction_offsets
                .get(pc)
                .copied()
                .filter(|offset| *offset != usize::MAX)
                .ok_or(Error::NoExec)?,
            BranchTarget::Error(status) => error_offsets
                .get(status as usize)
                .copied()
                .filter(|offset| *offset != usize::MAX)
                .ok_or(Error::Error)?,
            BranchTarget::CodeOffset(offset) => offset,
        };
        let displacement_end = fixup.displacement.checked_add(4).ok_or(Error::Overflow)?;
        let next_instruction = i64::try_from(displacement_end).map_err(|_| Error::Overflow)?;
        let target = i64::try_from(target).map_err(|_| Error::Overflow)?;
        let displacement = i32::try_from(target - next_instruction).map_err(|_| Error::Overflow)?;
        let bytes = code
            .get_mut(fixup.displacement..displacement_end)
            .ok_or(Error::Error)?;
        bytes.copy_from_slice(&displacement.to_le_bytes());
    }
    Ok(())
}

fn set_code_offset(fixups: &mut [Fixup], index: usize, offset: usize) -> Result<(), Error> {
    let fixup = fixups.get_mut(index).ok_or(Error::Error)?;
    fixup.target = BranchTarget::CodeOffset(offset);
    Ok(())
}

fn emit_jump(
    code: &mut Vec<u8>,
    target: BranchTarget,
    fixups: &mut Vec<Fixup>,
) -> Result<(), Error> {
    code.push(0xe9);
    let displacement = code.len();
    code.extend_from_slice(&[0; 4]);
    record_fixup(fixups, displacement, target)
}

fn emit_conditional_jump(
    code: &mut Vec<u8>,
    condition: u8,
    target: BranchTarget,
    fixups: &mut Vec<Fixup>,
) -> Result<usize, Error> {
    code.extend_from_slice(&[0x0f, condition]);
    let displacement = code.len();
    code.extend_from_slice(&[0; 4]);
    let index = fixups.len();
    record_fixup(fixups, displacement, target)?;
    Ok(index)
}

fn record_fixup(
    fixups: &mut Vec<Fixup>,
    displacement: usize,
    target: BranchTarget,
) -> Result<(), Error> {
    if fixups.len() == fixups.capacity() {
        fixups.try_reserve(1).map_err(|_| Error::NoMem)?;
    }
    fixups.push(Fixup {
        displacement,
        target,
    });
    Ok(())
}

fn emit_load_immediate(code: &mut Vec<u8>, register: u8, value: i64) -> Result<(), Error> {
    let rex = match register {
        REG_RAX | REG_RCX | REG_RDX => 0x48,
        REG_R8 | REG_R9 => 0x49,
        _ => return Err(Error::Inval),
    };
    code.extend_from_slice(&[rex, 0xb8 + (register & 7)]);
    code.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn emit_load_frame(code: &mut Vec<u8>, register: u8, offset: i32) -> Result<(), Error> {
    let rex = match register {
        REG_RAX | REG_RCX | REG_RDX => 0x48,
        REG_R8 | REG_R9 => 0x4c,
        _ => return Err(Error::Inval),
    };
    code.extend_from_slice(&[rex, 0x8b, 0x87 | ((register & 7) << 3)]);
    code.extend_from_slice(&offset.to_le_bytes());
    Ok(())
}

fn emit_store_frame(code: &mut Vec<u8>, register: u8, offset: i32) -> Result<(), Error> {
    let rex = match register {
        REG_RAX | REG_RCX | REG_RDX => 0x48,
        REG_R8 | REG_R9 => 0x4c,
        _ => return Err(Error::Inval),
    };
    code.extend_from_slice(&[rex, 0x89, 0x87 | ((register & 7) << 3)]);
    code.extend_from_slice(&offset.to_le_bytes());
    Ok(())
}

fn emit_store_frame_immediate(
    code: &mut Vec<u8>,
    offset: i32,
    value: i32,
) -> Result<(), Error> {
    code.extend_from_slice(&[0x48, 0xc7, 0x87]);
    code.extend_from_slice(&offset.to_le_bytes());
    code.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn emit_store_status(code: &mut Vec<u8>, status: u32) -> Result<(), Error> {
    let offset = i32::try_from(STATUS_OFFSET).map_err(|_| Error::Overflow)?;
    code.extend_from_slice(&[0xc7, 0x87]);
    code.extend_from_slice(&offset.to_le_bytes());
    code.extend_from_slice(&status.to_le_bytes());
    Ok(())
}

fn emit_increment_frame(code: &mut Vec<u8>, offset: usize) -> Result<(), Error> {
    let offset = i32::try_from(offset).map_err(|_| Error::Overflow)?;
    code.extend_from_slice(&[0x48, 0xff, 0x87]);
    code.extend_from_slice(&offset.to_le_bytes());
    Ok(())
}

fn emit_compare_frame_immediate(
    code: &mut Vec<u8>,
    offset: usize,
    value: i32,
) -> Result<(), Error> {
    let offset = i32::try_from(offset).map_err(|_| Error::Overflow)?;
    code.extend_from_slice(&[0x48, 0x81, 0xbf]);
    code.extend_from_slice(&offset.to_le_bytes());
    code.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn emit_compare_register_immediate(
    code: &mut Vec<u8>,
    register: u8,
    value: i8,
) -> Result<(), Error> {
    let rex = match register {
        REG_RAX | REG_RCX | REG_RDX => 0x48,
        REG_R8 | REG_R9 => 0x49,
        _ => return Err(Error::Inval),
    };
    code.extend_from_slice(&[rex, 0x83, 0xf8 | (register & 7), value as u8]);
    Ok(())
}

fn emit_compare_registers(code: &mut Vec<u8>, left: u8, right: u8) {
    emit_register_operation(code, 0x39, left, right);
}

fn emit_binary(code: &mut Vec<u8>, opcode: u8, destination: u8, source: u8) {
    emit_register_operation(code, opcode, destination, source);
}

fn emit_move_register(code: &mut Vec<u8>, destination: u8, source: u8) {
    emit_register_operation(code, 0x89, destination, source);
}

fn emit_register_operation(code: &mut Vec<u8>, opcode: u8, left: u8, right: u8) {
    let rex = 0x48 | (((right >> 3) & 1) << 2) | ((left >> 3) & 1);
    let modrm = 0xc0 | ((right & 7) << 3) | (left & 7);
    code.extend_from_slice(&[rex, opcode, modrm]);
}

fn emit_test_registers(code: &mut Vec<u8>, left: u8, right: u8) {
    emit_register_operation(code, 0x85, left, right);
}

fn slot_offset(base: usize, index: usize, field_offset: usize) -> Result<i32, Error> {
    let offset = index
        .checked_mul(size_of::<JitValue>())
        .and_then(|offset| base.checked_add(offset))
        .and_then(|offset| offset.checked_add(field_offset))
        .ok_or(Error::Overflow)?;
    i32::try_from(offset).map_err(|_| Error::Overflow)
}

fn synchronize_instruction_cache() {
    // SAFETY: MFENCE and CPUID serialize writes before executing the newly emitted x86_64 code.
    unsafe {
        core::arch::asm!(
            "mfence",
            "cpuid",
            inout("eax") 0u32 => _,
            out("ebx") _,
            out("ecx") _,
            out("edx") _,
            options(nostack),
        );
    }
}
