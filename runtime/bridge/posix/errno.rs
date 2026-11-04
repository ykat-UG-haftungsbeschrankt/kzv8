use crate::kernel::error::Error;

const fn as_errno(error: Error) -> i32 {
    -(error as i32)
}

pub(crate) const EAGAIN: i32 = as_errno(Error::Again);
pub(crate) const E2BIG: i32 = as_errno(Error::TooBig);
pub(crate) const EBADF: i32 = as_errno(Error::Badf);
pub(crate) const EFAULT: i32 = as_errno(Error::Fault);
pub(crate) const EINVAL: i32 = as_errno(Error::Inval);
pub(crate) const EILSEQ: i32 = as_errno(Error::Ilseq);
pub(crate) const ENAMETOOLONG: i32 = as_errno(Error::NameTooLong);
pub(crate) const ENOSPC: i32 = as_errno(Error::NoSpc);
pub(crate) const ENOTTY: i32 = as_errno(Error::NotTty);
pub(crate) const ENOSYS: i32 = as_errno(Error::NoSys);
pub(crate) const EAFNOSUPPORT: i32 = as_errno(Error::AfNoSupport);