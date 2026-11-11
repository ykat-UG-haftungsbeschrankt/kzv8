/*
 * Copyright (c) 2026 ykat UG (haftungsbeschraenkt) - All Rights Reserved
 *
 * Permission for non-commercial use is hereby granted,
 * free of charge, without warranty of any kind.
 */

use crate::kernel::error::Error;

const fn as_errno(error: Error) -> i32 {
	-(error as i32)
}

pub(crate) const EAGAIN: i32 = as_errno(Error::Again);
pub(crate) const EBUSY: i32 = as_errno(Error::Busy);
pub(crate) const EBADF: i32 = as_errno(Error::Badf);
pub(crate) const EEXIST: i32 = as_errno(Error::Exist);
pub(crate) const EFAULT: i32 = as_errno(Error::Fault);
pub(crate) const EFBIG: i32 = as_errno(Error::Fbig);
pub(crate) const E2BIG: i32 = as_errno(Error::TooBig);
pub(crate) const EDEADLK: i32 = as_errno(Error::Deadlk);
pub(crate) const EINVAL: i32 = as_errno(Error::Inval);
pub(crate) const EIO: i32 = as_errno(Error::Io);
pub(crate) const EACCES: i32 = as_errno(Error::Acces);
pub(crate) const EPERM: i32 = as_errno(Error::Error);
pub(crate) const EISDIR: i32 = as_errno(Error::IsDir);
pub(crate) const EMFILE: i32 = as_errno(Error::Mfile);
pub(crate) const ENAMETOOLONG: i32 = as_errno(Error::NameTooLong);
pub(crate) const ENOENT: i32 = as_errno(Error::NoEnt);
pub(crate) const ENODATA: i32 = as_errno(Error::NoData);
pub(crate) const ENOMEM: i32 = as_errno(Error::NoMem);
pub(crate) const ENOTDIR: i32 = as_errno(Error::NotDir);
pub(crate) const ENOTEMPTY: i32 = as_errno(Error::NotEmpty);
pub(crate) const ESRCH: i32 = as_errno(Error::Srch);
pub(crate) const ENOSYS: i32 = as_errno(Error::NoSys);
pub(crate) const ENOTTY: i32 = as_errno(Error::NotTty);
pub(crate) const EOVERFLOW: i32 = as_errno(Error::Overflow);
pub(crate) const ERANGE: i32 = as_errno(Error::Range);