/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[path = "support.rs"]
mod support;

use support::{write_u32, write_u64, zeroed, TestAddressSpace, TestFile};
use super::{MachOLoadedObject, ParsedMachO};
use crate::kernel::dl::{Error, ObjectKind};

#[cfg(target_arch = "x86_64")]
const CPU_TYPE: u32 = 0x0100_0007;
#[cfg(target_arch = "aarch64")]
const CPU_TYPE: u32 = 0x0100_000c;

fn executable_image() -> TestFile {
    let mut bytes = zeroed(0x200);
    bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
    write_u32(&mut bytes, 4, CPU_TYPE);
    write_u32(&mut bytes, 12, 2);
    write_u32(&mut bytes, 16, 2);
    write_u32(&mut bytes, 20, 96);
    write_u32(&mut bytes, 24, 0x0020_0000);

    let segment = 32;
    write_u32(&mut bytes, segment, 0x19);
    write_u32(&mut bytes, segment + 4, 72);
    bytes[segment + 8..segment + 14].copy_from_slice(b"__TEXT");
    write_u64(&mut bytes, segment + 24, 0x1_0000_0000);
    write_u64(&mut bytes, segment + 32, 0x1000);
    write_u64(&mut bytes, segment + 40, 0);
    write_u64(&mut bytes, segment + 48, 0x200);
    write_u32(&mut bytes, segment + 56, 5);
    write_u32(&mut bytes, segment + 60, 5);
    write_u32(&mut bytes, segment + 64, 0);

    let main = segment + 72;
    write_u32(&mut bytes, main, 0x8000_0028);
    write_u32(&mut bytes, main + 4, 24);
    write_u64(&mut bytes, main + 8, 0x100);
    TestFile::new(bytes)
}

#[test]
fn thin_macho_executable_maps_and_seals() {
    let source = executable_image();
    let parsed = ParsedMachO::parse(&source, ObjectKind::Executable).unwrap();
    assert_eq!(parsed.entry_virtual_address, Some(0x1_0000_0100));
    assert_eq!(parsed.mapping_length, 0x1000);

    let mut address_space = TestAddressSpace::default();
    address_space.reserve_base = Some(0x2000_0000);
    let mut object = MachOLoadedObject::load(
        &source,
        &mut address_space,
        ObjectKind::Executable,
        None,
    )
    .unwrap();
    assert_eq!(object.entry(), Some(0x2000_0100));
    assert_eq!(object.load_bias(), 0x2000_0000i128 - 0x1_0000_0000i128);
    object.seal(&mut address_space).unwrap();
    assert!(object.is_sealed());
}

#[test]
fn bind_stream_requires_done_opcode() {
    let mut bindings = Vec::new();
    assert_eq!(
        super::parse_bind_stream(&[0x10], &[], &[], false, false, &mut bindings),
        Err(Error::NoExec)
    );
    assert_eq!(
        super::parse_bind_stream(&[0], &[], &[], false, false, &mut bindings),
        Ok(())
    );
}