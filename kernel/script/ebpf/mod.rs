/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

// This is a first slice, not a full eBPF subsystem: maps, helpers, packet access,
// and runtime/syscall loading are not implemented yet.

use alloc::vec::Vec;
use crate::error::Error;

#[cfg(feature = "ebpf-jit")]
mod jit;

const INSTRUCTION_SIZE: usize = 8;
const MAX_INSTRUCTIONS: usize = 4096;
const REGISTER_COUNT: usize = 11;
const FRAME_POINTER: usize = 10;
const STACK_SIZE: usize = 512;

const CLASS_MASK: u8 = 0x07;
const SOURCE_REGISTER: u8 = 0x08;
const OPERATION_MASK: u8 = 0xf0;

const LD_IMMEDIATE_64: u8 = 0x18;
const LOAD_REGISTER: u8 = 0x01;
const STORE_IMMEDIATE: u8 = 0x02;
const STORE_REGISTER: u8 = 0x03;
const ALU32: u8 = 0x04;
const JUMP64: u8 = 0x05;
const JUMP32: u8 = 0x06;
const ALU64: u8 = 0x07;
const MEMORY_MODE: u8 = 0x60;

const OP_ADD: u8 = 0x00;
const OP_SUB: u8 = 0x10;
const OP_MUL: u8 = 0x20;
const OP_DIV: u8 = 0x30;
const OP_OR: u8 = 0x40;
const OP_AND: u8 = 0x50;
const OP_LSH: u8 = 0x60;
const OP_RSH: u8 = 0x70;
const OP_NEG: u8 = 0x80;
const OP_MOD: u8 = 0x90;
const OP_XOR: u8 = 0xa0;
const OP_MOV: u8 = 0xb0;
const OP_ARSH: u8 = 0xc0;

const OP_JA: u8 = 0x00;
const OP_JEQ: u8 = 0x10;
const OP_JGT: u8 = 0x20;
const OP_JGE: u8 = 0x30;
const OP_JSET: u8 = 0x40;
const OP_JNE: u8 = 0x50;
const OP_JSGT: u8 = 0x60;
const OP_JSGE: u8 = 0x70;
const OP_EXIT: u8 = 0x90;
const OP_JLT: u8 = 0xa0;
const OP_JLE: u8 = 0xb0;
const OP_JSLT: u8 = 0xc0;
const OP_JSLE: u8 = 0xd0;

#[derive(Clone, Copy)]
struct Instruction {
    opcode: u8,
    registers: u8,
    offset: i16,
    immediate: i32,
}

impl Instruction {
    fn decode(bytes: &[u8]) -> Self {
        Self {
            opcode: bytes[0],
            registers: bytes[1],
            offset: i16::from_le_bytes([bytes[2], bytes[3]]),
            immediate: i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }

    fn dst(self) -> usize {
        usize::from(self.registers & 0x0f)
    }

    fn src(self) -> usize {
        usize::from(self.registers >> 4)
    }
}

pub(crate) struct Program {
    //@FIXME: INDEXTYPE(instructions); dense program-counter-indexed bytecode array; Vec access is O(1), keep Vec.
    instructions: Vec<Instruction>,
    #[cfg(feature = "ebpf-jit")]
    jit: Option<jit::Code>,
}

impl Program {
    pub(crate) fn load(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error::Inval);
        }
        if bytes.len() > MAX_INSTRUCTIONS * INSTRUCTION_SIZE {
            return Err(Error::TooBig);
        }
        if bytes.len() % INSTRUCTION_SIZE != 0 {
            return Err(Error::Inval);
        }

        let mut instructions = Vec::with_capacity(bytes.len() / INSTRUCTION_SIZE);
        for bytes in bytes.chunks_exact(INSTRUCTION_SIZE) {
            instructions.push(Instruction::decode(bytes));
        }
        #[cfg(feature = "ebpf-jit")]
        {
            let mut program = Self::verify(instructions)?;
            program.jit = jit::Code::compile(&program.instructions);
            Ok(program)
        }
        #[cfg(not(feature = "ebpf-jit"))]
        Self::verify(instructions)
    }

    pub(crate) fn run(&self, input: u64) -> Result<u64, Error> {
        #[cfg(feature = "ebpf-jit")]
        if let Some(jit) = &self.jit {
            return Ok(jit.run(input));
        }

        self.run_interpreted(input)
    }

    fn run_interpreted(&self, input: u64) -> Result<u64, Error> {
        let mut registers = [0u64; REGISTER_COUNT];
        registers[1] = input;
        registers[FRAME_POINTER] = STACK_SIZE as u64;
        let mut stack = [0u8; STACK_SIZE];
        let mut pc = 0usize;

        loop {
            let instruction = self
                .instructions
                .get(pc)
                .copied()
                .ok_or(Error::Inval)?;

            if instruction.opcode == LD_IMMEDIATE_64 {
                let high = self
                    .instructions
                    .get(pc + 1)
                    .ok_or(Error::Inval)?
                    .immediate as u32 as u64;
                registers[instruction.dst()] =
                    (instruction.immediate as u32 as u64) | (high << 32);
                pc += 2;
                continue;
            }

            let class = instruction.opcode & CLASS_MASK;
            if class == ALU32 || class == ALU64 {
                execute_alu(instruction, &mut registers);
                pc += 1;
                continue;
            }

            if is_memory_instruction(instruction.opcode) {
                execute_memory(instruction, &mut registers, &mut stack)?;
                pc += 1;
                continue;
            }

            if instruction.opcode == (JUMP64 | OP_EXIT) {
                return Ok(registers[0]);
            }
            if is_unconditional_jump(instruction.opcode) {
                pc = runtime_jump_target(pc, instruction.offset)?;
                continue;
            }
            if is_conditional_jump(instruction.opcode) {
                if branch_taken(instruction, &registers) {
                    pc = runtime_jump_target(pc, instruction.offset)?;
                } else {
                    pc += 1;
                }
                continue;
            }

            return Err(Error::Inval);
        }
    }

    fn verify(instructions: Vec<Instruction>) -> Result<Self, Error> {
        if instructions.is_empty() {
            return Err(Error::Inval);
        }
        if instructions.len() > MAX_INSTRUCTIONS {
            return Err(Error::TooBig);
        }

        let mut continuation = alloc::vec![false; instructions.len()];
        for (pc, instruction) in instructions.iter().copied().enumerate() {
            if continuation[pc] {
                continue;
            }
            if instruction.opcode != LD_IMMEDIATE_64 {
                continue;
            }
            if instruction.dst() >= FRAME_POINTER
                || instruction.src() != 0
                || instruction.offset != 0
            {
                return Err(Error::Inval);
            }
            let Some(high) = instructions.get(pc + 1) else {
                return Err(Error::Inval);
            };
            if high.opcode != 0 || high.registers != 0 || high.offset != 0 {
                return Err(Error::Inval);
            }
            continuation[pc + 1] = true;
        }

        let mut targets = alloc::vec![None; instructions.len()];
        for (pc, instruction) in instructions.iter().copied().enumerate() {
            if continuation[pc] || instruction.opcode == LD_IMMEDIATE_64 {
                continue;
            }
            let class = instruction.opcode & CLASS_MASK;
            if class == ALU32 || class == ALU64 {
                validate_alu(instruction)?;
            } else if is_memory_instruction(instruction.opcode) {
                validate_memory(instruction)?;
            } else if instruction.opcode == (JUMP64 | OP_EXIT) {
                if instruction.registers != 0 || instruction.offset != 0 || instruction.immediate != 0 {
                    return Err(Error::Inval);
                }
            } else if is_unconditional_jump(instruction.opcode) {
                if instruction.dst() != 0 || instruction.src() != 0 || instruction.immediate != 0 {
                    return Err(Error::Inval);
                }
                targets[pc] = Some(verified_jump_target(
                    pc,
                    instruction.offset,
                    instructions.len(),
                    &continuation,
                )?);
            } else if is_conditional_jump(instruction.opcode) {
                validate_conditional_jump(instruction)?;
                targets[pc] = Some(verified_jump_target(
                    pc,
                    instruction.offset,
                    instructions.len(),
                    &continuation,
                )?);
            } else {
                return Err(Error::Inval);
            }
        }

        let mut terminates = alloc::vec![false; instructions.len()];
        for pc in (0..instructions.len()).rev() {
            if continuation[pc] {
                continue;
            }
            let instruction = instructions[pc];
            if instruction.opcode == (JUMP64 | OP_EXIT) {
                terminates[pc] = true;
            } else if is_unconditional_jump(instruction.opcode) {
                terminates[pc] = targets[pc].is_some_and(|target| terminates[target]);
            } else if is_conditional_jump(instruction.opcode) {
                let fallthrough = pc + 1;
                terminates[pc] = fallthrough < instructions.len()
                    && !continuation[fallthrough]
                    && targets[pc].is_some_and(|target| terminates[target])
                    && terminates[fallthrough];
            } else {
                let next = if instruction.opcode == LD_IMMEDIATE_64 {
                    pc + 2
                } else {
                    pc + 1
                };
                terminates[pc] = next < instructions.len()
                    && !continuation[next]
                    && terminates[next];
            }
        }

        if !terminates[0] {
            return Err(Error::Inval);
        }
        Ok(Self {
            instructions,
            #[cfg(feature = "ebpf-jit")]
            jit: None,
        })
    }
}

fn validate_alu(instruction: Instruction) -> Result<(), Error> {
    let operation = instruction.opcode & OPERATION_MASK;
    if !matches!(
        operation,
        OP_ADD | OP_SUB | OP_MUL | OP_DIV | OP_OR | OP_AND | OP_LSH | OP_RSH
            | OP_NEG | OP_MOD | OP_XOR | OP_MOV | OP_ARSH
    ) {
        return Err(Error::Inval);
    }
    if instruction.dst() >= FRAME_POINTER {
        return if instruction.dst() == FRAME_POINTER {
            Err(Error::Inval)
        } else {
            Err(Error::Inval)
        };
    }
    if instruction.offset != 0 {
        return Err(Error::Inval);
    }

    let register_source = instruction.opcode & SOURCE_REGISTER != 0;
    if operation == OP_NEG {
        if register_source || instruction.src() != 0 || instruction.immediate != 0 {
            return Err(Error::Inval);
        }
    } else if register_source {
        if instruction.src() >= REGISTER_COUNT || instruction.immediate != 0 {
            return Err(Error::Inval);
        }
    } else if instruction.src() != 0 {
        return Err(Error::Inval);
    }
    Ok(())
}

fn validate_memory(instruction: Instruction) -> Result<(), Error> {
    let class = instruction.opcode & CLASS_MASK;
    match class {
        LOAD_REGISTER => {
            if instruction.dst() >= FRAME_POINTER {
                return if instruction.dst() == FRAME_POINTER {
                    Err(Error::Inval)
                } else {
                    Err(Error::Inval)
                };
            }
            if instruction.src() >= REGISTER_COUNT || instruction.immediate != 0 {
                return Err(Error::Inval);
            }
        }
        STORE_IMMEDIATE => {
            if instruction.dst() >= REGISTER_COUNT || instruction.src() != 0 {
                return Err(Error::Inval);
            }
        }
        STORE_REGISTER => {
            if instruction.dst() >= REGISTER_COUNT || instruction.src() >= REGISTER_COUNT {
                return Err(Error::Inval);
            }
            if instruction.immediate != 0 {
                return Err(Error::Inval);
            }
        }
        _ => return Err(Error::Inval),
    }
    Ok(())
}

fn validate_conditional_jump(instruction: Instruction) -> Result<(), Error> {
    let operation = instruction.opcode & OPERATION_MASK;
    if !matches!(
        operation,
        OP_JEQ | OP_JGT | OP_JGE | OP_JSET | OP_JNE | OP_JSGT | OP_JSGE | OP_JLT | OP_JLE
            | OP_JSLT | OP_JSLE
    ) {
        return Err(Error::Inval);
    }
    if instruction.dst() >= REGISTER_COUNT {
        return Err(Error::Inval);
    }
    let register_source = instruction.opcode & SOURCE_REGISTER != 0;
    if register_source {
        if instruction.src() >= REGISTER_COUNT || instruction.immediate != 0 {
            return Err(Error::Inval);
        }
    } else if instruction.src() != 0 {
        return Err(Error::Inval);
    }
    Ok(())
}

fn verified_jump_target(
    pc: usize,
    offset: i16,
    instruction_count: usize,
    continuation: &[bool],
) -> Result<usize, Error> {
    if offset < 0 {
        return Err(Error::Inval);
    }
    let target = pc
        .checked_add(1)
        .and_then(|next| next.checked_add(offset as usize))
        .filter(|target| *target < instruction_count)
        .ok_or(Error::Inval)?;
    if continuation[target] {
        return Err(Error::Inval);
    }
    Ok(target)
}

fn runtime_jump_target(pc: usize, offset: i16) -> Result<usize, Error> {
    if offset < 0 {
        return Err(Error::Inval);
    }
    pc.checked_add(1)
        .and_then(|next| next.checked_add(offset as usize))
        .ok_or(Error::Inval)
}

fn is_unconditional_jump(opcode: u8) -> bool {
    opcode == (JUMP64 | OP_JA) || opcode == (JUMP32 | OP_JA)
}

fn is_conditional_jump(opcode: u8) -> bool {
    matches!(opcode & CLASS_MASK, JUMP64 | JUMP32)
        && matches!(
            opcode & OPERATION_MASK,
            OP_JEQ
                | OP_JGT
                | OP_JGE
                | OP_JSET
                | OP_JNE
                | OP_JSGT
                | OP_JSGE
                | OP_JLT
                | OP_JLE
                | OP_JSLT
                | OP_JSLE
        )
}

fn is_memory_instruction(opcode: u8) -> bool {
    matches!(opcode & CLASS_MASK, LOAD_REGISTER | STORE_IMMEDIATE | STORE_REGISTER)
        && opcode & 0xe0 == MEMORY_MODE
        && memory_width(opcode).is_some()
}

fn memory_width(opcode: u8) -> Option<usize> {
    if opcode & 0xe0 != MEMORY_MODE {
        return None;
    }
    match opcode & 0x18 {
        0x00 => Some(4),
        0x08 => Some(2),
        0x10 => Some(1),
        0x18 => Some(8),
        _ => None,
    }
}

fn execute_alu(instruction: Instruction, registers: &mut [u64; REGISTER_COUNT]) {
    let class = instruction.opcode & CLASS_MASK;
    let operation = instruction.opcode & OPERATION_MASK;
    let dst = instruction.dst();
    let source = if instruction.opcode & SOURCE_REGISTER != 0 {
        registers[instruction.src()]
    } else if class == ALU64 {
        instruction.immediate as i64 as u64
    } else {
        instruction.immediate as u32 as u64
    };

    if class == ALU32 {
        let left = registers[dst] as u32;
        let right = source as u32;
        let result = match operation {
            OP_ADD => left.wrapping_add(right),
            OP_SUB => left.wrapping_sub(right),
            OP_MUL => left.wrapping_mul(right),
            OP_DIV => if right == 0 { 0 } else { left / right },
            OP_OR => left | right,
            OP_AND => left & right,
            OP_LSH => left.wrapping_shl(right & 31),
            OP_RSH => left.wrapping_shr(right & 31),
            OP_NEG => left.wrapping_neg(),
            OP_MOD => if right == 0 { left } else { left % right },
            OP_XOR => left ^ right,
            OP_MOV => right,
            OP_ARSH => ((left as i32) >> (right & 31)) as u32,
            _ => left,
        };
        registers[dst] = u64::from(result);
    } else {
        let left = registers[dst];
        let right = source;
        registers[dst] = match operation {
            OP_ADD => left.wrapping_add(right),
            OP_SUB => left.wrapping_sub(right),
            OP_MUL => left.wrapping_mul(right),
            OP_DIV => if right == 0 { 0 } else { left / right },
            OP_OR => left | right,
            OP_AND => left & right,
            OP_LSH => left.wrapping_shl((right & 63) as u32),
            OP_RSH => left.wrapping_shr((right & 63) as u32),
            OP_NEG => left.wrapping_neg(),
            OP_MOD => if right == 0 { left } else { left % right },
            OP_XOR => left ^ right,
            OP_MOV => right,
            OP_ARSH => ((left as i64) >> (right & 63)) as u64,
            _ => left,
        };
    }
}

fn branch_taken(instruction: Instruction, registers: &[u64; REGISTER_COUNT]) -> bool {
    let class = instruction.opcode & CLASS_MASK;
    let operation = instruction.opcode & OPERATION_MASK;
    let left = registers[instruction.dst()];
    let right = if instruction.opcode & SOURCE_REGISTER != 0 {
        registers[instruction.src()]
    } else if class == JUMP32 {
        instruction.immediate as u32 as u64
    } else {
        instruction.immediate as i64 as u64
    };

    if class == JUMP32 {
        compare_u32(operation, left as u32, right as u32)
    } else {
        compare_u64(operation, left, right)
    }
}

fn compare_u32(operation: u8, left: u32, right: u32) -> bool {
    match operation {
        OP_JEQ => left == right,
        OP_JGT => left > right,
        OP_JGE => left >= right,
        OP_JSET => left & right != 0,
        OP_JNE => left != right,
        OP_JSGT => (left as i32) > (right as i32),
        OP_JSGE => (left as i32) >= (right as i32),
        OP_JLT => left < right,
        OP_JLE => left <= right,
        OP_JSLT => (left as i32) < (right as i32),
        OP_JSLE => (left as i32) <= (right as i32),
        _ => false,
    }
}

fn compare_u64(operation: u8, left: u64, right: u64) -> bool {
    match operation {
        OP_JEQ => left == right,
        OP_JGT => left > right,
        OP_JGE => left >= right,
        OP_JSET => left & right != 0,
        OP_JNE => left != right,
        OP_JSGT => (left as i64) > (right as i64),
        OP_JSGE => (left as i64) >= (right as i64),
        OP_JLT => left < right,
        OP_JLE => left <= right,
        OP_JSLT => (left as i64) < (right as i64),
        OP_JSLE => (left as i64) <= (right as i64),
        _ => false,
    }
}

fn execute_memory(
    instruction: Instruction,
    registers: &mut [u64; REGISTER_COUNT],
    stack: &mut [u8; STACK_SIZE],
) -> Result<(), Error> {
    let class = instruction.opcode & CLASS_MASK;
    let width = memory_width(instruction.opcode).ok_or(Error::Inval)?;
    let base_register = if class == LOAD_REGISTER {
        instruction.src()
    } else {
        instruction.dst()
    };
    let address = stack_address(registers[base_register], instruction.offset, width)?;

    match class {
        LOAD_REGISTER => {
            registers[instruction.dst()] = registers_load(stack, address, width)?;
            Ok(())
        }
        STORE_IMMEDIATE => {
            let value = instruction.immediate as i64 as u64;
            write_stack(stack, address, width, value)
        }
        STORE_REGISTER => write_stack(stack, address, width, registers[instruction.src()]),
        _ => Err(Error::Inval),
    }
}

fn registers_load(
    stack: &[u8; STACK_SIZE],
    address: usize,
    width: usize,
) -> Result<u64, Error> {
    let bytes = stack
        .get(address..address + width)
        .ok_or(Error::Fault)?;
    Ok(match width {
        1 => u64::from(bytes[0]),
        2 => u64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        4 => u64::from(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        8 => u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]),
        _ => return Err(Error::Inval),
    })
}

fn stack_address(base: u64, offset: i16, width: usize) -> Result<usize, Error> {
    let base = i64::try_from(base).map_err(|_| Error::Fault)?;
    let address = base
        .checked_add(i64::from(offset))
        .and_then(|address| usize::try_from(address).ok())
        .ok_or(Error::Fault)?;
    let end = address
        .checked_add(width)
        .ok_or(Error::Fault)?;
    if end > STACK_SIZE {
        return Err(Error::Fault);
    }
    Ok(address)
}

fn write_stack(
    stack: &mut [u8; STACK_SIZE],
    address: usize,
    width: usize,
    value: u64,
) -> Result<(), Error> {
    match width {
        1 => stack[address] = value as u8,
        2 => stack[address..address + 2].copy_from_slice(&(value as u16).to_le_bytes()),
        4 => stack[address..address + 4].copy_from_slice(&(value as u32).to_le_bytes()),
        8 => stack[address..address + 8].copy_from_slice(&value.to_le_bytes()),
        _ => return Err(Error::Inval),
    }
    Ok(())
}