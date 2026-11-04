#ifndef KZ_POSIX_SYS_UIO_H_
#define KZ_POSIX_SYS_UIO_H_

#include <stddef.h>
#include <sys/types.h>

struct iovec {
    void *iov_base;
    size_t iov_len;
};

#ifdef __cplusplus
extern "C" {
#endif

ssize_t readv(int file_descriptor, const struct iovec *vectors, int count);
ssize_t writev(int file_descriptor, const struct iovec *vectors, int count);

#ifdef __cplusplus
}
#endif

#endif