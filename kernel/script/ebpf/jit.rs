/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;
use core::mem::transmute;
use core::ptr::copy_nonoverlapping;

use super::{
    is_conditional_jump, is_memory_instruction, is_unconditional_jump, memory_width,
    stack_address, Instruction, ALU32, ALU64, CLASS_MASK, FRAME_POINTER, JUMP32, JUMP64,
    LD_IMMEDIATE_64, LOAD_REGISTER, OP_ADD, OP_AND, OP_ARSH, OP_DIV, OP_JEQ, OP_JGE, OP_JGT,
    OP_JLE, OP_JLT, OP_JNE, OP_JSET, OP_JSGE, OP_JSGT, OP_JSLE, OP_JSLT, OP_LSH, OP_MOD,
    OP_MOV, OP_MUL, OP_NEG, OP_OR, OP_RSH, OP_SUB, OP_XOR, OP_EXIT, REGISTER_COUNT,
    SOURCE_REGISTER, STACK_SIZE, STORE_IMMEDIATE, STORE_REGISTER,
};

pub(super) struct Code {
    address: usize,
    length: usize,
    entry: extern "C" fn(u64) -> u64,
}

impl Code {
    pub(super) fn compile(_instructions: &[Instruction]) -> Option<Self> {
        if !crate::kernel::memory::r#virtual::mmu_ready() {
            return None;
        }

        #[cfg(target_arch = "x86_64")]
        let machine_code = x86_64::compile(_instructions)?;
        #[cfg(target_arch = "aarch64")]
        let machine_code = aarch64::compile(_instructions)?;
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        return None;

        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        {
            let length = machine_code.len();
            let address = crate::kernel::memory::r#virtual::reserve(
                length,
                crate::kernel::memory::r#virtual::PAGE_SIZE,
                2,
            )?;

            // SAFETY: the source Vec is separate from the disjoint writable VM reservation.
            unsafe {
                copy_nonoverlapping(machine_code.as_ptr(), address as *mut u8, length);
            }
            if !crate::kernel::memory::r#virtual::set_permissions(address, length, 4) {
                let _released = crate::kernel::memory::r#virtual::release(address, length);
                return None;
            }
            if !synchronize_instruction_cache(address, length) {
                let _released = crate::kernel::memory::r#virtual::release(address, length);
                return None;
            }

            // SAFETY: the target emitter produces a complete C-ABI function and the range is RX.
            let entry = unsafe { transmute::<usize, extern "C" fn(u64) -> u64>(address) };
            Some(Self {
                address,
                length,
                entry,
            })
        }
    }

    pub(super) fn run(&self, input: u64) -> u64 {
        (self.entry)(input)
    }
}

impl Drop for Code {
    fn drop(&mut self) {
        let _released = crate::kernel::memory::r#virtual::release(self.address, self.length);
    }
}

fn jit_supported(instructions: &[Instruction]) -> bool {
    instructions.iter().copied().all(|instruction| {
        if !is_memory_instruction(instruction.opcode) {
            return true;
        }
        let class = instruction.opcode & CLASS_MASK;
        let base = if class == LOAD_REGISTER {
            instruction.src()
        } else {
            instruction.dst()
        };
        if base != FRAME_POINTER {
            return false;
        }
        let Some(width) = memory_width(instruction.opcode) else {
            return false;
        };
        let Ok(address) = stack_address(STACK_SIZE as u64, instruction.offset, width) else {
            return false;
        };
        #[cfg(target_arch = "aarch64")]
        if address % width != 0 {
            return false;
        }
        #[cfg(not(target_arch = "aarch64"))]
        let _ = address;
        true
    })
}

fn branch_target(pc: usize, instruction: Instruction) -> Option<usize> {
    if instruction.offset < 0 {
        return None;
    }
    pc.checked_add(1)?.checked_add(instruction.offset as usize)
}

#[cfg(target_arch = "x86_64")]
mod x86_64 {
    use super::*;

    const FRAME_SIZE: i32 =
        ((STACK_SIZE + REGISTER_COUNT * 8 + 15) & !15) as i32;
    const STACK_BASE_OFFSET: i32 = -(FRAME_SIZE);

    struct Fixup {
        displacement: usize,
        target_pc: usize,
    }

    struct Emitter {
        //@FIXME: INDEXTYPE(bytes); emitted machine-code buffer patched by byte offset; Vec indexing is O(1), keep Vec.
        bytes: Vec<u8>,
        //@FIXME: INDEXTYPE(fixups); append-then-iterate relocation patch list; keep Vec.
        fixups: Vec<Fixup>,
    }

    impl Emitter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                fixups: Vec::new(),
            }
        }

        fn emit(&mut self, bytes: &[u8]) {
            self.bytes.extend_from_slice(bytes);
        }

        fn emit_i32(&mut self, value: i32) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }

        fn emit_u32(&mut self, value: u32) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }

        fn emit_u64(&mut self, value: u64) {
            self.bytes.extend_from_slice(&value.to_le_bytes());
        }

        fn register_displacement(register: usize) -> i32 {
            -((register as i32 + 1) * 8)
        }

        fn load_slot(&mut self, destination: u8, register: usize) {
            self.emit(&[0x48, 0x8b, 0x85 | (destination << 3)]);
            self.emit_i32(Self::register_displacement(register));
        }

        fn store_slot(&mut self, source: u8, register: usize) {
            self.emit(&[0x48, 0x89, 0x85 | (source << 3)]);
            self.emit_i32(Self::register_displacement(register));
        }

        fn load_immediate(&mut self, destination: u8, value: u64) {
            self.emit(&[0x48, 0xb8 + destination]);
            self.emit_u64(value);
        }

        fn load_source(&mut self, instruction: Instruction, class: u8) {
            if instruction.opcode & SOURCE_REGISTER != 0 {
                self.load_slot(1, instruction.src());
            } else if class == ALU64 {
                self.emit(&[0x48, 0xc7, 0xc1]);
                self.emit_i32(instruction.immediate);
            } else {
                self.emit(&[0xb9]);
                self.emit_u32(instruction.immediate as u32);
            }
        }

        fn initialize(&mut self) {
            for register in 0..REGISTER_COUNT {
                self.emit(&[0x48, 0xc7, 0x85]);
                self.emit_i32(Self::register_displacement(register));
                self.emit_u32(0);
            }
            for address in (0..STACK_SIZE).step_by(8) {
                self.emit(&[0x48, 0xc7, 0x85]);
                self.emit_i32(STACK_BASE_OFFSET + address as i32);
                self.emit_u32(0);
            }
            self.store_slot(7, 1);
            self.emit(&[0x48, 0xc7, 0x85]);
            self.emit_i32(Self::register_displacement(FRAME_POINTER));
            self.emit_u32(STACK_SIZE as u32);
        }

        fn emit_alu(&mut self, instruction: Instruction) -> bool {
            let class = instruction.opcode & CLASS_MASK;
            let operation = instruction.opcode & 0xf0;
            let is_32_bit = class == ALU32;
            self.load_slot(0, instruction.dst());
            if operation != OP_NEG {
                self.load_source(instruction, class);
            }

            match operation {
                OP_ADD => self.emit(if is_32_bit { &[0x01, 0xc8] } else { &[0x48, 0x01, 0xc8] }),
                OP_SUB => self.emit(if is_32_bit { &[0x29, 0xc8] } else { &[0x48, 0x29, 0xc8] }),
                OP_MUL => self.emit(if is_32_bit {
                    &[0x0f, 0xaf, 0xc1]
                } else {
                    &[0x48, 0x0f, 0xaf, 0xc1]
                }),
                OP_DIV => self.emit_division(is_32_bit, false),
                OP_OR => self.emit(if is_32_bit { &[0x09, 0xc8] } else { &[0x48, 0x09, 0xc8] }),
                OP_AND => self.emit(if is_32_bit { &[0x21, 0xc8] } else { &[0x48, 0x21, 0xc8] }),
                OP_LSH => self.emit(if is_32_bit { &[0xd3, 0xe0] } else { &[0x48, 0xd3, 0xe0] }),
                OP_RSH => self.emit(if is_32_bit { &[0xd3, 0xe8] } else { &[0x48, 0xd3, 0xe8] }),
                OP_NEG => self.emit(if is_32_bit { &[0xf7, 0xd8] } else { &[0x48, 0xf7, 0xd8] }),
                OP_MOD => self.emit_division(is_32_bit, true),
                OP_XOR => self.emit(if is_32_bit { &[0x31, 0xc8] } else { &[0x48, 0x31, 0xc8] }),
                OP_MOV => self.emit(if is_32_bit { &[0x89, 0xc8] } else { &[0x48, 0x89, 0xc8] }),
                OP_ARSH => self.emit(if is_32_bit { &[0xd3, 0xf8] } else { &[0x48, 0xd3, 0xf8] }),
                _ => return false,
            }
            self.store_slot(0, instruction.dst());
            true
        }

        fn emit_division(&mut self, is_32_bit: bool, modulo: bool) {
            self.emit(if is_32_bit {
                &[0x85, 0xc9]
            } else {
                &[0x48, 0x85, 0xc9]
            });
            let zero_jump = self.bytes.len();
            self.emit(&[0x74, 0]);
            self.emit(&[0x31, 0xd2]);
            self.emit(if is_32_bit {
                &[0xf7, 0xf1]
            } else {
                &[0x48, 0xf7, 0xf1]
            });
            if modulo {
                self.emit(if is_32_bit { &[0x89, 0xd0] } else { &[0x48, 0x89, 0xd0] });
                let zero_target = self.bytes.len();
                if is_32_bit {
                    self.emit(&[0x89, 0xc0]);
                }
                self.patch_short_jump(zero_jump, zero_target);
            } else {
                let end_jump = self.bytes.len();
                self.emit(&[0xeb, 0]);
                let zero_target = self.bytes.len();
                self.emit(&[0x31, 0xc0]);
                let end = self.bytes.len();
                self.patch_short_jump(zero_jump, zero_target);
                self.patch_short_jump(end_jump, end);
            }
        }

        fn patch_short_jump(&mut self, opcode_offset: usize, target: usize) {
            let displacement = (target as isize) - (opcode_offset as isize + 2);
            self.bytes[opcode_offset + 1] = displacement as i8 as u8;
        }

        fn emit_conditional_jump(&mut self, instruction: Instruction, target_pc: usize) -> bool {
            let class = instruction.opcode & CLASS_MASK;
            let operation = instruction.opcode & 0xf0;
            let is_32_bit = class == JUMP32;
            self.load_slot(0, instruction.dst());
            if instruction.opcode & SOURCE_REGISTER != 0 {
                self.load_slot(1, instruction.src());
            } else if is_32_bit {
                self.emit(&[0xb9]);
                self.emit_u32(instruction.immediate as u32);
            } else {
                self.emit(&[0x48, 0xc7, 0xc1]);
                self.emit_i32(instruction.immediate);
            }
            self.emit(if operation == OP_JSET {
                if is_32_bit { &[0x85, 0xc8] } else { &[0x48, 0x85, 0xc8] }
            } else if is_32_bit {
                &[0x39, 0xc8]
            } else {
                &[0x48, 0x39, 0xc8]
            });

            let condition = match operation {
                OP_JEQ => 0x84,
                OP_JNE | OP_JSET => 0x85,
                OP_JGT => 0x87,
                OP_JGE => 0x83,
                OP_JLT => 0x82,
                OP_JLE => 0x86,
                OP_JSGT => 0x8f,
                OP_JSGE => 0x8d,
                OP_JSLT => 0x8c,
                OP_JSLE => 0x8e,
                _ => return false,
            };
            self.emit(&[0x0f, condition]);
            let displacement = self.bytes.len();
            self.emit_i32(0);
            self.fixups.push(Fixup {
                displacement,
                target_pc,
            });
            true
        }

        fn emit_memory(&mut self, instruction: Instruction) -> Option<()> {
            let class = instruction.opcode & CLASS_MASK;
            let width = memory_width(instruction.opcode)?;
            let stack_index = stack_address(STACK_SIZE as u64, instruction.offset, width).ok()?;
            let displacement = STACK_BASE_OFFSET + stack_index as i32;

            match class {
                LOAD_REGISTER => {
                    match width {
                        1 => self.emit(&[0x0f, 0xb6, 0x85]),
                        2 => self.emit(&[0x0f, 0xb7, 0x85]),
                        4 => self.emit(&[0x8b, 0x85]),
                        8 => self.emit(&[0x48, 0x8b, 0x85]),
                        _ => return None,
                    }
                    self.emit_i32(displacement);
                    self.store_slot(0, instruction.dst());
                }
                STORE_IMMEDIATE | STORE_REGISTER => {
                    if class == STORE_IMMEDIATE {
                        self.load_immediate(0, instruction.immediate as i64 as u64);
                    } else {
                        self.load_slot(0, instruction.src());
                    }
                    match width {
                        1 => self.emit(&[0x88, 0x85]),
                        2 => self.emit(&[0x66, 0x89, 0x85]),
                        4 => self.emit(&[0x89, 0x85]),
                        8 => self.emit(&[0x48, 0x89, 0x85]),
                        _ => return None,
                    }
                    self.emit_i32(displacement);
                }
                _ => return None,
            }
            Some(())
        }

        fn emit_epilogue(&mut self) {
            self.load_slot(0, 0);
            self.emit(&[0x48, 0x89, 0xec, 0x5d, 0xc3]);
        }

        fn patch_branches(&mut self, instruction_offsets: &[usize]) -> Option<()> {
            for fixup in &self.fixups {
                let target = *instruction_offsets.get(fixup.target_pc)?;
                if target == usize::MAX {
                    return None;
                }
                let next_instruction = fixup.displacement.checked_add(4)?;
                let relative = i32::try_from(target as i64 - next_instruction as i64).ok()?;
                self.bytes[fixup.displacement..next_instruction]
                    .copy_from_slice(&relative.to_le_bytes());
            }
            Some(())
        }
    }

    pub(super) fn compile(instructions: &[Instruction]) -> Option<Vec<u8>> {
        if !jit_supported(instructions) {
            return None;
        }
        let mut emitter = Emitter::new();
        let mut instruction_offsets = alloc::vec![usize::MAX; instructions.len()];
        emitter.emit(&[0x55, 0x48, 0x89, 0xe5, 0x48, 0x81, 0xec]);
        emitter.emit_u32(FRAME_SIZE as u32);
        emitter.initialize();

        for (pc, instruction) in instructions.iter().copied().enumerate() {
            if instruction.opcode == 0 {
                continue;
            }
            instruction_offsets[pc] = emitter.bytes.len();
            if instruction.opcode == LD_IMMEDIATE_64 {
                let high = instructions.get(pc + 1)?.immediate as u32 as u64;
                let value = (instruction.immediate as u32 as u64) | (high << 32);
                emitter.load_immediate(0, value);
                emitter.store_slot(0, instruction.dst());
            } else if matches!(instruction.opcode & CLASS_MASK, ALU32 | ALU64) {
                if !emitter.emit_alu(instruction) {
                    return None;
                }
            } else if is_memory_instruction(instruction.opcode) {
                emitter.emit_memory(instruction)?;
            } else if instruction.opcode == (JUMP64 | OP_EXIT) {
                emitter.emit_epilogue();
            } else if is_unconditional_jump(instruction.opcode) {
                let target_pc = branch_target(pc, instruction)?;
                emitter.emit(&[0xe9]);
                let displacement = emitter.bytes.len();
                emitter.emit_i32(0);
                emitter.fixups.push(Fixup {
                    displacement,
                    target_pc,
                });
            } else if is_conditional_jump(instruction.opcode) {
                let target_pc = branch_target(pc, instruction)?;
                if !emitter.emit_conditional_jump(instruction, target_pc) {
                    return None;
                }
            } else {
                return None;
            }
        }
        emitter.patch_branches(&instruction_offsets)?;
        Some(emitter.bytes)
    }
}

#[cfg(target_arch = "aarch64")]
mod aarch64 {
    use super::*;

    const FRAME_SIZE: usize = (STACK_SIZE + REGISTER_COUNT * 8 + 15) & !15;

    struct Fixup {
        instruction_offset: usize,
        target_pc: usize,
        condition: Option<u32>,
    }

    struct Emitter {
        //@FIXME: INDEXTYPE(bytes); emitted machine-code buffer patched by byte offset; Vec indexing is O(1), keep Vec.
        bytes: Vec<u8>,
        //@FIXME: INDEXTYPE(fixups); append-then-iterate relocation patch list; keep Vec.
        fixups: Vec<Fixup>,
    }

    impl Emitter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                fixups: Vec::new(),
            }
        }

        fn emit(&mut self, instruction: u32) {
            self.bytes.extend_from_slice(&instruction.to_le_bytes());
        }

        fn load_slot(&mut self, destination: u32, register: usize) {
            let offset = STACK_SIZE + register * 8;
            self.emit(
                0xf940_0000
                    | ((offset as u32 / 8) << 10)
                    | (31 << 5)
                    | destination,
            );
        }

        fn store_slot(&mut self, source: u32, register: usize) {
            let offset = STACK_SIZE + register * 8;
            self.emit(
                0xf900_0000
                    | ((offset as u32 / 8) << 10)
                    | (31 << 5)
                    | source,
            );
        }

        fn move_immediate(&mut self, destination: u32, value: u64) {
            self.emit(0xd280_0000 | ((value as u32 & 0xffff) << 5) | destination);
            for halfword in 1..4 {
                let immediate = ((value >> (halfword * 16)) & 0xffff) as u32;
                if immediate != 0 {
                    self.emit(
                        0xf280_0000
                            | ((halfword as u32) << 21)
                            | (immediate << 5)
                            | destination,
                    );
                }
            }
        }

        fn initialize(&mut self) {
            for offset in (0..FRAME_SIZE).step_by(8) {
                self.emit(
                    0xf900_001f | ((offset as u32 / 8) << 10) | (31 << 5),
                );
            }
            self.store_slot(0, 1);
            self.move_immediate(9, STACK_SIZE as u64);
            self.store_slot(9, FRAME_POINTER);
        }

        fn encode_rrr(base: u32, destination: u32, left: u32, right: u32) -> u32 {
            base | (right << 16) | (left << 5) | destination
        }

        fn emit_alu(&mut self, instruction: Instruction) -> bool {
            let class = instruction.opcode & CLASS_MASK;
            let operation = instruction.opcode & 0xf0;
            let is_32_bit = class == ALU32;
            self.load_slot(9, instruction.dst());
            if instruction.opcode & SOURCE_REGISTER != 0 {
                self.load_slot(11, instruction.src());
            } else if operation != OP_NEG {
                let value = if is_32_bit {
                    instruction.immediate as u32 as u64
                } else {
                    instruction.immediate as i64 as u64
                };
                self.move_immediate(11, value);
            }

            let operation_base = match operation {
                OP_ADD => Some(if is_32_bit { 0x0b00_0000 } else { 0x8b00_0000 }),
                OP_SUB => Some(if is_32_bit { 0x4b00_0000 } else { 0xcb00_0000 }),
                OP_MUL => Some(if is_32_bit { 0x1b00_7c00 } else { 0x9b00_7c00 }),
                OP_DIV => Some(if is_32_bit { 0x1ac0_0800 } else { 0x9ac0_0800 }),
                OP_OR => Some(if is_32_bit { 0x2a00_0000 } else { 0xaa00_0000 }),
                OP_AND => Some(if is_32_bit { 0x0a00_0000 } else { 0x8a00_0000 }),
                OP_LSH => Some(if is_32_bit { 0x1ac0_2000 } else { 0x9ac0_2000 }),
                OP_RSH => Some(if is_32_bit { 0x1ac0_2400 } else { 0x9ac0_2400 }),
                OP_XOR => Some(if is_32_bit { 0x4a00_0000 } else { 0xca00_0000 }),
                OP_ARSH => Some(if is_32_bit { 0x1ac0_2800 } else { 0x9ac0_2800 }),
                OP_NEG => Some(if is_32_bit { 0x4b00_0000 } else { 0xcb00_0000 }),
                OP_MOV => Some(if is_32_bit { 0x2a00_0000 } else { 0xaa00_0000 }),
                OP_MOD => None,
                _ => return false,
            };

            if operation == OP_MOD {
                let divide = if is_32_bit { 0x1ac0_0800 } else { 0x9ac0_0800 };
                let multiply_subtract = if is_32_bit { 0x1b00_8000 } else { 0x9b00_8000 };
                self.emit(Self::encode_rrr(divide, 12, 9, 11));
                self.emit(multiply_subtract | (11 << 16) | (9 << 10) | (12 << 5) | 9);
            } else {
                let Some(operation_base) = operation_base else {
                    return false;
                };
                if operation == OP_NEG {
                    self.emit(Self::encode_rrr(operation_base, 9, 31, 9));
                } else if operation == OP_MOV {
                    self.emit(Self::encode_rrr(operation_base, 9, 31, 11));
                } else {
                    self.emit(Self::encode_rrr(operation_base, 9, 9, 11));
                }
            }
            self.store_slot(9, instruction.dst());
            true
        }

        fn emit_conditional_jump(&mut self, instruction: Instruction, target_pc: usize) -> bool {
            let class = instruction.opcode & CLASS_MASK;
            let operation = instruction.opcode & 0xf0;
            let is_32_bit = class == JUMP32;
            self.load_slot(9, instruction.dst());
            if instruction.opcode & SOURCE_REGISTER != 0 {
                self.load_slot(11, instruction.src());
            } else {
                let value = if is_32_bit {
                    instruction.immediate as u32 as u64
                } else {
                    instruction.immediate as i64 as u64
                };
                self.move_immediate(11, value);
            }
            if operation == OP_JSET {
                self.emit(if is_32_bit {
                    Self::encode_rrr(0x6a00_001f, 31, 9, 11)
                } else {
                    Self::encode_rrr(0xea00_001f, 31, 9, 11)
                });
            } else {
                self.emit(if is_32_bit {
                    Self::encode_rrr(0x6b00_001f, 31, 9, 11)
                } else {
                    Self::encode_rrr(0xeb00_001f, 31, 9, 11)
                });
            }

            let condition = match operation {
                OP_JEQ => 0,
                OP_JNE | OP_JSET => 1,
                OP_JGE => 2,
                OP_JLT => 3,
                OP_JGT => 8,
                OP_JLE => 9,
                OP_JSGE => 10,
                OP_JSLT => 11,
                OP_JSGT => 12,
                OP_JSLE => 13,
                _ => return false,
            };
            let instruction_offset = self.bytes.len();
            self.emit(0x5400_0000 | condition);
            self.fixups.push(Fixup {
                instruction_offset,
                target_pc,
                condition: Some(condition),
            });
            true
        }

        fn emit_memory(&mut self, instruction: Instruction) -> Option<()> {
            let class = instruction.opcode & CLASS_MASK;
            let width = memory_width(instruction.opcode)?;
            let stack_index = stack_address(STACK_SIZE as u64, instruction.offset, width).ok()?;
            self.emit(0x9100_0000 | ((stack_index as u32) << 10) | (31 << 5) | 13);
            match class {
                LOAD_REGISTER => {
                    let opcode = match width {
                        1 => 0x3940_0000,
                        2 => 0x7940_0000,
                        4 => 0xb940_0000,
                        8 => 0xf940_0000,
                        _ => return None,
                    };
                    self.emit(opcode | (13 << 5) | 9);
                    self.store_slot(9, instruction.dst());
                }
                STORE_IMMEDIATE | STORE_REGISTER => {
                    if class == STORE_IMMEDIATE {
                        self.move_immediate(9, instruction.immediate as i64 as u64);
                    } else {
                        self.load_slot(9, instruction.src());
                    }
                    let opcode = match width {
                        1 => 0x3900_0000,
                        2 => 0x7900_0000,
                        4 => 0xb900_0000,
                        8 => 0xf900_0000,
                        _ => return None,
                    };
                    self.emit(opcode | (13 << 5) | 9);
                }
                _ => return None,
            }
            Some(())
        }

        fn emit_epilogue(&mut self) {
            self.load_slot(0, 0);
            self.emit(0x9100_03bf);
            self.emit(0xa8c1_7bfd);
            self.emit(0xd65f_03c0);
        }

        fn patch_branches(&mut self, instruction_offsets: &[usize]) -> Option<()> {
            for fixup in &self.fixups {
                let target = *instruction_offsets.get(fixup.target_pc)?;
                if target == usize::MAX {
                    return None;
                }
                let delta = target as i64 - fixup.instruction_offset as i64;
                if delta % 4 != 0 {
                    return None;
                }
                let words = delta / 4;
                let instruction = if let Some(condition) = fixup.condition {
                    if !(-(1 << 18)..(1 << 18)).contains(&words) {
                        return None;
                    }
                    0x5400_0000 | (((words as u32) & 0x7ffff) << 5) | condition
                } else {
                    if !(-(1 << 25)..(1 << 25)).contains(&words) {
                        return None;
                    }
                    0x1400_0000 | ((words as u32) & 0x03ff_ffff)
                };
                self.bytes[fixup.instruction_offset..fixup.instruction_offset + 4]
                    .copy_from_slice(&instruction.to_le_bytes());
            }
            Some(())
        }
    }

    pub(super) fn compile(instructions: &[Instruction]) -> Option<Vec<u8>> {
        if !jit_supported(instructions) {
            return None;
        }
        let mut emitter = Emitter::new();
        let mut instruction_offsets = alloc::vec![usize::MAX; instructions.len()];
        emitter.emit(0xa9bf_7bfd);
        emitter.emit(0x9100_03fd);
        emitter.emit(0xd100_0000 | ((FRAME_SIZE as u32) << 10) | (31 << 5) | 31);
        emitter.initialize();

        for (pc, instruction) in instructions.iter().copied().enumerate() {
            if instruction.opcode == 0 {
                continue;
            }
            instruction_offsets[pc] = emitter.bytes.len();
            if instruction.opcode == LD_IMMEDIATE_64 {
                let high = instructions.get(pc + 1)?.immediate as u32 as u64;
                let value = (instruction.immediate as u32 as u64) | (high << 32);
                emitter.move_immediate(9, value);
                emitter.store_slot(9, instruction.dst());
            } else if matches!(instruction.opcode & CLASS_MASK, ALU32 | ALU64) {
                if !emitter.emit_alu(instruction) {
                    return None;
                }
            } else if is_memory_instruction(instruction.opcode) {
                emitter.emit_memory(instruction)?;
            } else if instruction.opcode == (JUMP64 | OP_EXIT) {
                emitter.emit_epilogue();
            } else if is_unconditional_jump(instruction.opcode) {
                let target_pc = branch_target(pc, instruction)?;
                let instruction_offset = emitter.bytes.len();
                emitter.emit(0x1400_0000);
                emitter.fixups.push(Fixup {
                    instruction_offset,
                    target_pc,
                    condition: None,
                });
            } else if is_conditional_jump(instruction.opcode) {
                let target_pc = branch_target(pc, instruction)?;
                if !emitter.emit_conditional_jump(instruction, target_pc) {
                    return None;
                }
            } else {
                return None;
            }
        }
        emitter.patch_branches(&instruction_offsets)?;
        Some(emitter.bytes)
    }
}

#[cfg(target_arch = "x86_64")]
fn synchronize_instruction_cache(_address: usize, _length: usize) -> bool {
    // SAFETY: CPUID serializes instruction fetching after the JIT code has been written.
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
    true
}

#[cfg(target_arch = "aarch64")]
fn synchronize_instruction_cache(address: usize, length: usize) -> bool {
    let Some(end) = address.checked_add(length) else {
        return false;
    };
    // SAFETY: clean the emitted code's data-cache lines and invalidate inner-shareable I-caches.
    unsafe {
        let ctr_el0: u64;
        core::arch::asm!(
            "mrs {ctr_el0}, ctr_el0",
            ctr_el0 = out(reg) ctr_el0,
            options(nomem, nostack, preserves_flags),
        );
        let data_line_size = 4usize << ((ctr_el0 >> 16) & 0xf);
        let mut line = address & !(data_line_size - 1);
        while line < end {
            core::arch::asm!(
                "dc cvau, {address}",
                address = in(reg) line,
                options(nostack, preserves_flags),
            );
            line += data_line_size;
        }
        core::arch::asm!("dsb ish", options(nostack, preserves_flags));
        core::arch::asm!(
            "ic ialluis",
            "dsb ish",
            "isb",
            options(nostack, preserves_flags),
        );
    }
    true
}