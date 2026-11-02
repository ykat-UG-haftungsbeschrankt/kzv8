#ifndef KZ_ERRNO_H
#define KZ_ERRNO_H

#ifndef EINVAL
#define EINVAL 22
#endif
#ifndef ENOMEM
#define ENOMEM 12
#endif
#ifndef EACCES
#define EACCES 13
#endif
#ifndef EFAULT
#define EFAULT 14
#endif
#ifndef EBUSY
#define EBUSY 16
#endif
#ifndef EEXIST
#define EEXIST 17
#endif
#ifndef ENOTDIR
#define ENOTDIR 20
#endif
#ifndef EISDIR
#define EISDIR 21
#endif
#ifndef EMFILE
#define EMFILE 24
#endif
#ifndef EFBIG
#define EFBIG 27
#endif
#ifndef ENOSPC
#define ENOSPC 28
#endif
#ifndef ESPIPE
#define ESPIPE 29
#endif
#ifndef ENAMETOOLONG
#define ENAMETOOLONG 36
#endif
#ifndef ENOTEMPTY
#define ENOTEMPTY 39
#endif
#ifndef EOVERFLOW
#define EOVERFLOW 75
#endif
#ifndef ENOSYS
#define ENOSYS 38
#endif
#ifndef EIO
#define EIO 5
#endif
#ifndef EBADF
#define EBADF 9
#endif
#ifndef ENOENT
#define ENOENT 2
#endif
#ifndef EAGAIN
#define EAGAIN 11
#endif
#ifndef ERANGE
#define ERANGE 34
#endif
#ifndef EINVAL
#define EINVAL 22
#endif

#ifdef __cplusplus
extern "C" {
#endif

int *__errno_location(void);

#ifdef __cplusplus
}
#endif

#define errno (*__errno_location())

#endif