/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

pub(crate) mod passthru;
pub(crate) mod virtio;

pub(crate) struct GuestMemory<'a> {
    bytes: &'a mut [u8],
}

impl<'a> GuestMemory<'a> {
    pub(crate) fn new(bytes: &'a mut [u8]) -> Self {
        Self { bytes }
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn contains(&self, address: u64, length: usize) -> bool {
        self.range(address, length).is_some()
    }

    pub(crate) fn read(&self, address: u64, output: &mut [u8]) -> bool {
        let Some(range) = self.range(address, output.len()) else {
            return false;
        };
        output.copy_from_slice(&self.bytes[range]);
        true
    }

    pub(crate) fn write(&mut self, address: u64, input: &[u8]) -> bool {
        let Some(range) = self.range(address, input.len()) else {
            return false;
        };
        self.bytes[range].copy_from_slice(input);
        true
    }

    pub(crate) fn read_u16(&self, address: u64) -> Option<u16> {
        let mut bytes = [0; 2];
        self.read(address, &mut bytes)
            .then(|| u16::from_le_bytes(bytes))
    }

    pub(crate) fn read_u32(&self, address: u64) -> Option<u32> {
        let mut bytes = [0; 4];
        self.read(address, &mut bytes)
            .then(|| u32::from_le_bytes(bytes))
    }

    pub(crate) fn read_u64(&self, address: u64) -> Option<u64> {
        let mut bytes = [0; 8];
        self.read(address, &mut bytes)
            .then(|| u64::from_le_bytes(bytes))
    }

    pub(crate) fn write_u16(&mut self, address: u64, value: u16) -> bool {
        self.write(address, &value.to_le_bytes())
    }

    pub(crate) fn write_u32(&mut self, address: u64, value: u32) -> bool {
        self.write(address, &value.to_le_bytes())
    }

    fn range(&self, address: u64, length: usize) -> Option<core::ops::Range<usize>> {
        let start = usize::try_from(address).ok()?;
        let end = start.checked_add(length)?;
        (end <= self.bytes.len()).then_some(start..end)
    }
}