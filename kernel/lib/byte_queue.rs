/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use alloc::vec::Vec;

use crate::kernel::error::Error;

pub(crate) struct ByteQueue {
    //@FIXME: INDEXTYPE(bytes); ring-buffer backing array; Vec provides O(1) indexed access, keep Vec.
    bytes: Vec<u8>,
    head: usize,
    length: usize,
}

impl ByteQueue {
    pub(crate) fn new(capacity: usize) -> Result<Self, Error> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| Error::NoMem)?;
        bytes.resize(capacity, 0);
        Ok(Self {
            bytes,
            head: 0,
            length: 0,
        })
    }

    pub(crate) fn available(&self) -> usize {
        self.bytes.len() - self.length
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub(crate) fn push(&mut self, byte: u8) -> bool {
        if self.length == self.bytes.len() {
            return false;
        }
        let tail = (self.head + self.length) % self.bytes.len();
        self.bytes[tail] = byte;
        self.length += 1;
        true
    }

    pub(crate) fn push_slice(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > self.available() {
            return false;
        }
        for byte in bytes {
            let _ = self.push(*byte);
        }
        true
    }

    pub(crate) fn pop_into(&mut self, output: &mut [u8]) -> usize {
        let count = output.len().min(self.length);
        for byte in &mut output[..count] {
            *byte = self.bytes[self.head];
            self.head = (self.head + 1) % self.bytes.len();
        }
        self.length -= count;
        if self.length == 0 {
            self.head = 0;
        }
        count
    }

    pub(crate) fn clear(&mut self) {
        self.head = 0;
        self.length = 0;
    }
}