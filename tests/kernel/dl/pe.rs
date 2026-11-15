/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[path = "support.rs"]
mod support;

use support::{write_u16, write_u32, write_u64, zeroed, TestAddressSpace, TestFile};
use super::{ParsedPe, PeLoadedObject};
use crate::kernel::dl::ObjectKind;

fn executable_image() -> TestFile {
    const IMAGE_BASE: u64 = 0x1_4000_0000;
    let mut bytes = zeroed(0x400);
    bytes[..2].copy_from_slice(b"MZ");
    write_u32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    write_u16(&mut bytes, 0x84, 0x8664);
    write_u16(&mut bytes, 0x86, 1);
    write_u16(&mut bytes, 0x94, 0xf0);
    write_u16(&mut bytes, 0x96, 0x0002);

    let optional = 0x98;
    write_u16(&mut bytes, optional, 0x20b);
    write_u32(&mut bytes, optional + 16, 0x1000);
    write_u64(&mut bytes, optional + 24, IMAGE_BASE);
    write_u32(&mut bytes, optional + 32, 0x1000);
    write_u32(&mut bytes, optional + 36, 0x200);
    write_u32(&mut bytes, optional + 56, 0x2000);
    write_u32(&mut bytes, optional + 60, 0x200);
    write_u32(&mut bytes, optional + 108, 16);

    let section = optional + 0xf0;
    bytes[section..section + 5].copy_from_slice(b".text");
    write_u32(&mut bytes, section + 8, 0x1000);
    write_u32(&mut bytes, section + 12, 0x1000);
    write_u32(&mut bytes, section + 16, 0x200);
    write_u32(&mut bytes, section + 20, 0x200);
    write_u32(&mut bytes, section + 36, 0x6000_0000);
    TestFile::new(bytes)
}

fn executable_with_readonly_iat() -> TestFile {
    const IMAGE_BASE: u64 = 0x1_4000_0000;
    let mut bytes = zeroed(0x600);
    bytes[..2].copy_from_slice(b"MZ");
    write_u32(&mut bytes, 0x3c, 0x80);
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    write_u16(&mut bytes, 0x84, 0x8664);
    write_u16(&mut bytes, 0x86, 2);
    write_u16(&mut bytes, 0x94, 0xf0);
    write_u16(&mut bytes, 0x96, 0x0002);

    let optional = 0x98;
    write_u16(&mut bytes, optional, 0x20b);
    write_u32(&mut bytes, optional + 16, 0x1000);
    write_u64(&mut bytes, optional + 24, IMAGE_BASE);
    write_u32(&mut bytes, optional + 32, 0x1000);
    write_u32(&mut bytes, optional + 36, 0x200);
    write_u32(&mut bytes, optional + 56, 0x3000);
    write_u32(&mut bytes, optional + 60, 0x200);
    write_u32(&mut bytes, optional + 108, 16);
    write_u32(&mut bytes, optional + 120, 0x2000);
    write_u32(&mut bytes, optional + 124, 40);

    let text_section = optional + 0xf0;
    bytes[text_section..text_section + 5].copy_from_slice(b".text");
    write_u32(&mut bytes, text_section + 8, 0x1000);
    write_u32(&mut bytes, text_section + 12, 0x1000);
    write_u32(&mut bytes, text_section + 16, 0x200);
    write_u32(&mut bytes, text_section + 20, 0x200);
    write_u32(&mut bytes, text_section + 36, 0x6000_0020);

    let data_section = text_section + 40;
    bytes[data_section..data_section + 6].copy_from_slice(b".rdata");
    write_u32(&mut bytes, data_section + 8, 0x1000);
    write_u32(&mut bytes, data_section + 12, 0x2000);
    write_u32(&mut bytes, data_section + 16, 0x200);
    write_u32(&mut bytes, data_section + 20, 0x400);
    write_u32(&mut bytes, data_section + 36, 0x4000_0040);

    write_u32(&mut bytes, 0x400, 0x2030);
    write_u32(&mut bytes, 0x40c, 0x2040);
    write_u32(&mut bytes, 0x410, 0x2050);
    write_u64(&mut bytes, 0x430, 0x2060);
    bytes[0x440..0x44c].copy_from_slice(b"library.dll\0");
    write_u64(&mut bytes, 0x450, 0x2060);
    bytes[0x462..0x46b].copy_from_slice(b"imported\0");
    TestFile::new(bytes)
}

#[test]
fn pe_import_address_table_can_be_in_readonly_data() {
    let source = executable_with_readonly_iat();
    let parsed = ParsedPe::parse(&source, ObjectKind::Executable).unwrap();

    assert_eq!(parsed.imports.len(), 1);
    assert_eq!(parsed.imports[0].target_rva, 0x2050);
}

#[test]
fn pe32_plus_executable_maps_and_seals() {
    let source = executable_image();
    let parsed = ParsedPe::parse(&source, ObjectKind::Executable).unwrap();
    assert_eq!(parsed.entry_rva, 0x1000);
    assert_eq!(parsed.mapping_length, 0x2000);

    let mut address_space = TestAddressSpace::default();
    let mut object = PeLoadedObject::load(
        &source,
        &mut address_space,
        ObjectKind::Executable,
        None,
    )
    .unwrap();
    assert_eq!(object.entry(), Some(0x1_4000_1000));
    object.seal(&mut address_space).unwrap();
    assert!(object.is_sealed());
    assert!(address_space
        .protections()
        .iter()
        .all(|(_, _, permissions)| permissions & 0x06 != 0x06));
}