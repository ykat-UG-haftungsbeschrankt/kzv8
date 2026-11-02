/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

#[derive(Debug)]
pub(crate) enum Operation<'a> {
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

#[derive(Debug)]
pub(crate) struct Request<'a> {
    pub(crate) sector: u64,
    pub(crate) operation: Operation<'a>,
}

impl<'a> Request<'a> {
    pub(crate) const fn read(sector: u64, output: &'a mut [u8]) -> Self {
        Self {
            sector,
            operation: Operation::Read(output),
        }
    }

    pub(crate) const fn write(sector: u64, input: &'a [u8]) -> Self {
        Self {
            sector,
            operation: Operation::Write(input),
        }
    }
}
