/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::dl::{AddressSpace, Error, FileSource, Permissions};
use crate::kernel::dl::PermissionsApi as _;

pub(crate) struct TestFile {
    bytes: Vec<u8>,
}

impl TestFile {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl FileSource for TestFile {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, Error> {
        let offset = usize::try_from(offset).map_err(|_| Error::Overflow)?;
        if offset >= self.bytes.len() {
            return Ok(0);
        }
        let count = buffer.len().min(self.bytes.len() - offset);
        buffer[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        Ok(count)
    }
}

#[derive(Default)]
pub(crate) struct TestAddressSpace {
    base: usize,
    bytes: Vec<u8>,
    protections: Vec<(usize, usize, u8)>,
    pub(crate) released: bool,
    pub(crate) reserve_base: Option<usize>,
}

impl TestAddressSpace {
    fn offset(&self, address: usize, length: usize) -> Result<usize, Error> {
        let offset = address.checked_sub(self.base).ok_or(Error::NoMem)?;
        let end = offset.checked_add(length).ok_or(Error::Overflow)?;
        if end > self.bytes.len() {
            return Err(Error::NoMem);
        }
        Ok(offset)
    }

    pub(crate) fn read_bytes(&self, address: usize, length: usize) -> Result<&[u8], Error> {
        let offset = self.offset(address, length)?;
        Ok(&self.bytes[offset..offset + length])
    }

    pub(crate) fn protections(&self) -> &[(usize, usize, u8)] {
        &self.protections
    }
}

impl AddressSpace for TestAddressSpace {
    fn reserve(
        &mut self,
        preferred_address: Option<usize>,
        length: usize,
        alignment: usize,
        alignment_offset: usize,
        fixed: bool,
    ) -> Result<usize, Error> {
        let base = if fixed {
            preferred_address.ok_or(Error::NoMem)?
        } else {
            self.reserve_base
                .or(preferred_address)
                .unwrap_or(0x1000_0000)
        };
        if base % alignment != alignment_offset || length == 0 {
            return Err(Error::NoMem);
        }
        self.base = base;
        self.bytes.clear();
        self.bytes
            .try_reserve_exact(length)
            .map_err(|_| Error::NoMem)?;
        self.bytes.resize(length, 0);
        self.protections.clear();
        self.released = false;
        Ok(base)
    }

    fn zero(&mut self, address: usize, length: usize) -> Result<(), Error> {
        let offset = self.offset(address, length)?;
        self.bytes[offset..offset + length].fill(0);
        Ok(())
    }

    fn read(&self, address: usize, buffer: &mut [u8]) -> Result<(), Error> {
        let offset = self.offset(address, buffer.len())?;
        buffer.copy_from_slice(&self.bytes[offset..offset + buffer.len()]);
        Ok(())
    }

    fn write(&mut self, address: usize, buffer: &[u8]) -> Result<(), Error> {
        let offset = self.offset(address, buffer.len())?;
        self.bytes[offset..offset + buffer.len()].copy_from_slice(buffer);
        Ok(())
    }

    fn protect(
        &mut self,
        address: usize,
        length: usize,
        permissions: Permissions,
    ) -> Result<(), Error> {
        self.offset(address, length)?;
        self.protections
            .try_reserve(1)
            .map_err(|_| Error::NoMem)?;
        self.protections
            .push((address, length, permissions.bits()));
        Ok(())
    }

    fn release(&mut self, _address: usize, _length: usize) {
        self.released = true;
        self.bytes.clear();
    }
}

pub(crate) fn zeroed(length: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.resize(length, 0);
    bytes
}

pub(crate) fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(crate) fn write_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}