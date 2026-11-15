/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

const STATUS_UNSUCCESSFUL: u32 = 0xc000_0001;
const STATUS_NOT_IMPLEMENTED: u32 = 0xc000_0002;
const STATUS_ACCESS_VIOLATION: u32 = 0xc000_0005;
const STATUS_INVALID_HANDLE: u32 = 0xc000_0008;
const STATUS_INVALID_PARAMETER: u32 = 0xc000_000d;
const STATUS_NO_SUCH_DEVICE: u32 = 0xc000_000e;
const STATUS_END_OF_FILE: u32 = 0xc000_0011;
const STATUS_NO_MEMORY: u32 = 0xc000_0017;
const STATUS_ACCESS_DENIED: u32 = 0xc000_0022;
const STATUS_SHARING_VIOLATION: u32 = 0xc000_0043;
const STATUS_LOCK_NOT_GRANTED: u32 = 0xc000_0055;
const STATUS_INVALID_IMAGE_FORMAT: u32 = 0xc000_007b;
const STATUS_DISK_FULL: u32 = 0xc000_007f;
const STATUS_MEDIA_WRITE_PROTECTED: u32 = 0xc000_00a2;
const STATUS_PIPE_DISCONNECTED: u32 = 0xc000_00b0;
const STATUS_IO_TIMEOUT: u32 = 0xc000_00b5;
const STATUS_FILE_IS_A_DIRECTORY: u32 = 0xc000_00ba;
const STATUS_NOT_SUPPORTED: u32 = 0xc000_00bb;
const STATUS_DEVICE_BUSY: u32 = 0x8000_0011;
const STATUS_INTEGER_OVERFLOW: u32 = 0xc000_0095;
const STATUS_NOT_SAME_DEVICE: u32 = 0xc000_00d4;
const STATUS_PIPE_BROKEN: u32 = 0xc000_014b;
const STATUS_CANCELLED: u32 = 0xc000_0120;
const STATUS_POSSIBLE_DEADLOCK: u32 = 0xc000_0194;
const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xc000_0034;
const STATUS_OBJECT_NAME_COLLISION: u32 = 0xc000_0035;
const STATUS_TOO_MANY_OPENED_FILES: u32 = 0xc000_011f;
const STATUS_NOT_FOUND: u32 = 0xc000_0225;
const STATUS_RETRY: u32 = 0xc000_022d;
const STATUS_NOT_A_DIRECTORY: u32 = 0xc000_0103;
const STATUS_DIRECTORY_NOT_EMPTY: u32 = 0xc000_0101;
const STATUS_NAME_TOO_LONG: u32 = 0xc000_0106;
const STATUS_TOO_MANY_LINKS: u32 = 0xc000_0265;
const STATUS_REPARSE_POINT_NOT_RESOLVED: u32 = 0xc000_0280;
const STATUS_FILE_TOO_LARGE: u32 = 0xc000_0904;
const STATUS_INVALID_DEVICE_REQUEST: u32 = 0xc000_0010;
const STATUS_IO_DEVICE_ERROR: u32 = 0xc000_0185;
pub(crate) const STATUS_NO_YIELD_PERFORMED: u32 = 0x4000_0024;

pub(crate) const fn ntstatus(error: Error) -> u32 {
    match error {
        Error::Success => 0,
        Error::Error => STATUS_UNSUCCESSFUL,
        Error::NoEnt | Error::Idrm | Error::NoLink => STATUS_OBJECT_NAME_NOT_FOUND,
        Error::Srch | Error::Child => STATUS_NOT_FOUND,
        Error::Intr | Error::Restart => STATUS_CANCELLED,
        Error::Io | Error::RemoteIo | Error::Uclean => STATUS_IO_DEVICE_ERROR,
        Error::Nxio | Error::NoDev => STATUS_NO_SUCH_DEVICE,
        Error::TooBig | Error::Inval | Error::Min | Error::Max => STATUS_INVALID_PARAMETER,
        Error::NoExec => STATUS_INVALID_IMAGE_FORMAT,
        Error::Badf | Error::BadFd => STATUS_INVALID_HANDLE,
        Error::Again | Error::WouldBlock => STATUS_RETRY,
        Error::NoMem => STATUS_NO_MEMORY,
        Error::Acces | Error::Perm => STATUS_ACCESS_DENIED,
        Error::Fault => STATUS_ACCESS_VIOLATION,
        Error::NotBlk | Error::NotTty | Error::NotSock => STATUS_INVALID_DEVICE_REQUEST,
        Error::Busy => STATUS_DEVICE_BUSY,
        Error::Exist => STATUS_OBJECT_NAME_COLLISION,
        Error::Xdev => STATUS_NOT_SAME_DEVICE,
        Error::NotDir => STATUS_NOT_A_DIRECTORY,
        Error::IsDir => STATUS_FILE_IS_A_DIRECTORY,
        Error::Nfile | Error::Mfile => STATUS_TOO_MANY_OPENED_FILES,
        Error::TxtBsy => STATUS_SHARING_VIOLATION,
        Error::Fbig => STATUS_FILE_TOO_LARGE,
        Error::NoSpc => STATUS_DISK_FULL,
        Error::Spipe => STATUS_PIPE_DISCONNECTED,
        Error::Rofs => STATUS_MEDIA_WRITE_PROTECTED,
        Error::Mlink => STATUS_TOO_MANY_LINKS,
        Error::Pipe | Error::Strpipe => STATUS_PIPE_BROKEN,
        Error::Dom | Error::Range => STATUS_INVALID_PARAMETER,
        Error::Deadlk | Error::Deadlock => STATUS_POSSIBLE_DEADLOCK,
        Error::NameTooLong => STATUS_NAME_TOO_LONG,
        Error::Nolck => STATUS_LOCK_NOT_GRANTED,
        Error::NoSys => STATUS_NOT_IMPLEMENTED,
        Error::OpNotSupp | Error::NoProtoOpt | Error::ProtoNoSupport
        | Error::SockTypeNoSupport | Error::PfNoSupport | Error::AfNoSupport => {
            STATUS_NOT_SUPPORTED
        }
        Error::NotEmpty => STATUS_DIRECTORY_NOT_EMPTY,
        Error::Loop => STATUS_REPARSE_POINT_NOT_RESOLVED,
        Error::NoData => STATUS_END_OF_FILE,
        Error::Time | Error::TimedOut => STATUS_IO_TIMEOUT,
        Error::Overflow => STATUS_INTEGER_OVERFLOW,
        _ => STATUS_UNSUCCESSFUL,
    }
}

#[cfg(test)]
#[path = "../../../tests/kernel/syscall/windows/error.rs"]
mod tests;