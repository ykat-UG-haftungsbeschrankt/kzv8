/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use super::*;

fn relocation_object(
    position_independent: bool,
    permissions: u32,
    needed: Vec<Vec<u8>>,
    relocation: Relocation,
) -> ElfLoadedObject {
    ElfLoadedObject {
        kind: ObjectKind::Executable,
        mapping_base: 0x10_000,
        mapping_length: 0x4_000,
        load_bias: 0x10_000,
        entry: Some(0x11_000),
        load_ranges: alloc::vec![LoadRange {
            start: 0x1_000,
            end: 0x5_000,
            permissions,
        }],
        relro_segments: alloc::vec![ProgramHeader {
            kind: PT_GNU_RELRO,
            flags: PF_READ,
            offset: 0,
            virtual_address: 0x2_000,
            file_size: 0x1_000,
            memory_size: 0x1_000,
            alignment: 1,
        }],
        needed,
        symbols: Vec::new(),
        relocations: alloc::vec![relocation],
        position_independent,
        self_relocation_prepared: false,
        state: ObjectState::RelocationPending,
    }
}

fn relative_relocation(offset: u64) -> Relocation {
    Relocation {
        offset,
        symbol_index: 0,
        kind: relative_relocation_type(),
        addend: Some(0x1234),
    }
}

#[test]
fn self_relocation_accepts_writable_relative_relocations() {
    let mut object = relocation_object(
        true,
        PF_READ | PF_WRITE,
        Vec::new(),
        relative_relocation(0x2_000),
    );

    assert!(object.prepare_for_self_relocation().is_ok());
    assert!(matches!(object.state, ObjectState::ReadyToSeal));
    assert!(object.self_relocation_prepared);
}

#[test]
fn self_relocation_rejects_unsupported_images_and_targets() {
    let mut fixed = relocation_object(
        false,
        PF_READ | PF_WRITE,
        Vec::new(),
        relative_relocation(0x2_000),
    );
    assert!(fixed.prepare_for_self_relocation().is_err());

    let mut symbol_relocation = relocation_object(
        true,
        PF_READ | PF_WRITE,
        Vec::new(),
        Relocation {
            symbol_index: 1,
            ..relative_relocation(0x2_000)
        },
    );
    assert!(symbol_relocation.prepare_for_self_relocation().is_err());

    let mut read_only_target = relocation_object(
        true,
        PF_READ,
        Vec::new(),
        relative_relocation(0x2_000),
    );
    assert!(read_only_target
        .prepare_for_self_relocation()
        .is_err());

    let mut dependent = relocation_object(
        true,
        PF_READ | PF_WRITE,
        alloc::vec![alloc::vec![b'l', b'i', b'b', b'e', b'x', b'a', b'm', b'p', b'l', b'e']],
        relative_relocation(0x2_000),
    );
    assert!(dependent.prepare_for_self_relocation().is_err());
}